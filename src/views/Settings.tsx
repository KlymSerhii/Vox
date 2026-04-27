import {
  createMemo,
  createSignal,
  For,
  type JSX,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import {
  cancelDownload,
  checkForUpdates,
  deleteMode,
  deleteModel,
  diagnoseHotkeys,
  downloadModel,
  duplicateMode,
  exportMode,
  getActiveHotkey,
  getLaunchAtLogin,
  getSettings,
  importModeJson,
  listModels,
  listModes,
  onModelCanceled,
  onModelDone,
  onModelError,
  onModelProgress,
  onSettingsChanged,
  openAccessibilitySettings,
  replayOnboarding,
  resetDefaultModes,
  revealDevBinaryPath,
  saveMode,
  setLaunchAtLogin,
  setSettings,
  type CaseStyle,
  type DevBinaryInfo,
  type DiagnoseHotkeys,
  type HotkeyDiagnostic,
  type ModelStatus,
  type PunctuationStyle,
  type Settings,
  type UpdateCheckResult,
  type VoiceMode,
} from "../lib/ipc";

type Tab =
  | "general"
  | "hotkeys"
  | "models"
  | "modes"
  | "dictionary"
  | "privacy"
  | "about";

const TABS: { id: Tab; label: string; icon: string }[] = [
  { id: "general", label: "General", icon: "sliders" },
  { id: "hotkeys", label: "Hotkeys", icon: "keyboard" },
  { id: "models", label: "Models", icon: "cube" },
  { id: "modes", label: "Modes", icon: "modes" },
  { id: "dictionary", label: "Dictionary", icon: "book" },
  { id: "privacy", label: "Privacy", icon: "lock" },
  { id: "about", label: "About", icon: "info" },
];

interface HotkeyOption {
  value: string;
  label: string;
  /** When true the picker renders a small ⚠︎ glyph and a tooltip
   * explaining the runtime fallback. Right-Option falls into this
   * bucket because `tauri-plugin-global-shortcut` reports an
   * "Unknown scancode for AltRight" error on most macOS keymaps and
   * the dictation hotkey would silently fail to bind. */
  warn?: string;
}

const CANCEL_HOTKEY_OPTIONS: HotkeyOption[] = [
  { value: "Escape", label: "Esc" },
  { value: "F13", label: "F13" },
  { value: "F14", label: "F14" },
  { value: "F15", label: "F15" },
];

const MODE_CYCLE_HOTKEY_OPTIONS: HotkeyOption[] = [
  { value: "Alt+M", label: "⌥M" },
  { value: "Alt+Shift+M", label: "⌥⇧M" },
  { value: "CommandOrControl+Shift+M", label: "⌘⇧M" },
  { value: "F13", label: "F13" },
  { value: "F14", label: "F14" },
  { value: "F15", label: "F15" },
  { value: "", label: "Off" },
];

const PUNCTUATION_OPTIONS: { value: PunctuationStyle; label: string }[] = [
  { value: "Aggressive", label: "Aggressive" },
  { value: "Standard", label: "Standard" },
  { value: "Minimal", label: "Minimal" },
  { value: "None", label: "None (strip)" },
];

const CASE_OPTIONS: { value: CaseStyle; label: string }[] = [
  { value: "Sentence", label: "Sentence" },
  { value: "Title", label: "Title" },
  { value: "Lower", label: "Lowercase" },
  { value: "AsSpoken", label: "As spoken" },
];

const LANGUAGE_OPTIONS: { value: string; label: string }[] = [
  { value: "auto", label: "Auto (detect)" },
  { value: "en", label: "English" },
  { value: "es", label: "Spanish" },
  { value: "fr", label: "French" },
  { value: "de", label: "German" },
  { value: "it", label: "Italian" },
  { value: "pt", label: "Portuguese" },
  { value: "ja", label: "Japanese" },
  { value: "zh", label: "Chinese" },
  { value: "ru", label: "Russian" },
  { value: "ar", label: "Arabic" },
];

export default function SettingsView() {
  const [tab, setTab] = createSignal<Tab>("general");
  const [loaded, setLoaded] = createSignal(false);
  const [state, setState] = createSignal<Settings | null>(null);
  const [savedFlash, setSavedFlash] = createSignal(false);

  let unlistenSettings: (() => void) | null = null;
  // Bumped each time *we* save from inside this window, so the
  // settings:changed event handler can ignore the echo it triggers.
  let lastSelfSaveAt = 0;

  onMount(async () => {
    try {
      const s = await getSettings();
      try {
        const osLaunch = await getLaunchAtLogin();
        if (osLaunch !== s.launch_at_login) {
          s.launch_at_login = osLaunch;
        }
      } catch (e) {
        console.warn("getLaunchAtLogin failed:", e);
      }
      setState(s);
    } finally {
      setLoaded(true);
    }

    // Keep this window in sync when settings change elsewhere — for
    // example when the user flips dictation on/off from the tray
    // popover. The `lastSelfSaveAt` guard suppresses the echo from our
    // own `update()` calls so we don't churn through a redundant
    // setState (which would force every controlled input to re-render
    // mid-typing).
    unlistenSettings = await onSettingsChanged((next) => {
      if (Date.now() - lastSelfSaveAt < 500) return;
      setState(next);
    });
  });

  onCleanup(() => {
    unlistenSettings?.();
  });

  async function update(patch: Partial<Settings>) {
    const cur = state();
    if (!cur) return;
    const next = { ...cur, ...patch } as Settings;
    setState(next);
    try {
      lastSelfSaveAt = Date.now();
      await setSettings(next);
      setSavedFlash(true);
      setTimeout(() => setSavedFlash(false), 240);
    } catch (err) {
      console.error("Save failed:", err);
    }
  }

  return (
    <div class="flex h-screen w-screen bg-[var(--color-bg)] text-[var(--color-ink)]">
      <Show when={loaded() && state()} fallback={<Loader />}>
        {(s) => (
          <>
            <Sidebar tab={tab()} onTab={setTab} />
            <main class="flex-1 overflow-y-auto scrollbar-thin px-8 py-6">
              <Header tab={tab()} savedFlash={savedFlash()} />
              <Show when={tab() === "general"}>
                <GeneralTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "hotkeys"}>
                <HotkeysTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "models"}>
                <ModelsTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "modes"}>
                <ModesTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "dictionary"}>
                <DictionaryTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "privacy"}>
                <PrivacyTab s={s()} />
              </Show>
              <Show when={tab() === "about"}>
                <AboutTab />
              </Show>
            </main>
          </>
        )}
      </Show>
    </div>
  );
}

function Loader() {
  return (
    <div class="flex h-screen w-screen items-center justify-center text-[var(--color-ink-mute)]">
      Loading…
    </div>
  );
}

function Sidebar(props: { tab: Tab; onTab: (t: Tab) => void }) {
  return (
    <aside class="flex h-full w-[200px] flex-col border-r border-[var(--color-border)] bg-[rgba(24,24,27,0.6)] px-3 py-4">
      <div class="mb-4 px-2">
        <div class="text-[15px] font-semibold">Vox</div>
        <div class="text-[11px] text-[var(--color-ink-mute)]">
          Talk to any app.
        </div>
      </div>
      <nav class="flex flex-col gap-0.5">
        <For each={TABS}>
          {(t) => (
            <button
              type="button"
              onClick={() => props.onTab(t.id)}
              class="flex items-center gap-2 rounded-[6px] px-3 py-1.5 text-left text-[13px] transition-colors"
              classList={{
                "bg-white/6 text-[var(--color-ink)]": props.tab === t.id,
                "text-[var(--color-ink-mute)] hover:bg-white/4":
                  props.tab !== t.id,
              }}
            >
              <TabIcon name={t.icon} active={props.tab === t.id} />
              <span>{t.label}</span>
            </button>
          )}
        </For>
      </nav>
      <div class="mt-auto px-3 text-center text-[11px] text-[var(--color-ink-faint)]">
        Vox 1.0.0
      </div>
    </aside>
  );
}

function TabIcon(props: { name: string; active: boolean }) {
  const fill = () =>
    props.active ? "var(--color-accent)" : "var(--color-ink-mute)";
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke={fill()}
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.name === "sliders" && (
        <>
          <line x1="4" y1="21" x2="4" y2="14" />
          <line x1="4" y1="10" x2="4" y2="3" />
          <line x1="12" y1="21" x2="12" y2="12" />
          <line x1="12" y1="8" x2="12" y2="3" />
          <line x1="20" y1="21" x2="20" y2="16" />
          <line x1="20" y1="12" x2="20" y2="3" />
          <line x1="1" y1="14" x2="7" y2="14" />
          <line x1="9" y1="8" x2="15" y2="8" />
          <line x1="17" y1="16" x2="23" y2="16" />
        </>
      )}
      {props.name === "keyboard" && (
        <>
          <rect x="2" y="6" width="20" height="12" rx="2" />
          <line x1="6" y1="10" x2="6" y2="10" />
          <line x1="10" y1="10" x2="10" y2="10" />
          <line x1="14" y1="10" x2="14" y2="10" />
          <line x1="18" y1="10" x2="18" y2="10" />
          <line x1="7" y1="14" x2="17" y2="14" />
        </>
      )}
      {props.name === "key" && (
        <>
          <circle cx="8" cy="15" r="4" />
          <line x1="10.8" y1="12.2" x2="21" y2="2" />
          <line x1="17" y1="6" x2="21" y2="10" />
        </>
      )}
      {props.name === "book" && (
        <>
          <path d="M4 19.5V4a2 2 0 0 1 2-2h13v18H6a2 2 0 0 0-2 2" />
          <line x1="8" y1="7" x2="15" y2="7" />
        </>
      )}
      {props.name === "lock" && (
        <>
          <rect x="4" y="11" width="16" height="10" rx="2" />
          <path d="M8 11V7a4 4 0 0 1 8 0v4" />
        </>
      )}
      {props.name === "info" && (
        <>
          <circle cx="12" cy="12" r="9" />
          <line x1="12" y1="16" x2="12" y2="12" />
          <line x1="12" y1="8" x2="12.01" y2="8" />
        </>
      )}
      {props.name === "cube" && (
        <>
          <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
          <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
          <line x1="12" y1="22.08" x2="12" y2="12" />
        </>
      )}
      {props.name === "modes" && (
        <>
          <circle cx="12" cy="12" r="9" />
          <path d="M12 3v18" />
          <path d="M3 12h18" />
        </>
      )}
    </svg>
  );
}

function Header(props: { tab: Tab; savedFlash: boolean }) {
  const title = createMemo(
    () => TABS.find((t) => t.id === props.tab)?.label ?? "",
  );
  return (
    <header class="mb-5 flex items-center justify-between border-b border-[var(--color-border)] pb-3">
      <h1 class="text-[17px] font-semibold">{title()}</h1>
      <Show when={props.savedFlash}>
        <span class="text-[11px] text-[var(--color-success)]">✓ Saved</span>
      </Show>
    </header>
  );
}

function Card(props: { children: any }) {
  return (
    <div class="mb-5 overflow-hidden rounded-[10px] bg-[var(--color-surface-raised)] ring-1 ring-[var(--color-border)]">
      {props.children}
    </div>
  );
}

function Row(props: {
  label: string;
  hint?: string;
  children: any;
  last?: boolean;
}) {
  return (
    <div
      class="flex items-center justify-between gap-4 px-4 py-3"
      classList={{
        "border-b border-[var(--color-border)]": !props.last,
      }}
    >
      <div class="flex flex-col">
        <span class="text-[13px] text-[var(--color-ink)]">{props.label}</span>
        <Show when={props.hint}>
          <span class="text-[11px] text-[var(--color-ink-mute)]">
            {props.hint}
          </span>
        </Show>
      </div>
      <div class="flex items-center gap-2">{props.children}</div>
    </div>
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

// -------- Tabs --------

function GeneralTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  async function toggleLaunchAtLogin(v: boolean) {
    try {
      await setLaunchAtLogin(v);
      props.update({ launch_at_login: v });
    } catch (err) {
      console.error("Launch-at-login change failed:", err);
    }
  }

  return (
    <>
      <Card>
        <Row
          label="Launch at login"
          hint="Vox will start quietly in your menu bar."
          last
        >
          <Toggle
            checked={props.s.launch_at_login}
            onChange={(v) => void toggleLaunchAtLogin(v)}
          />
        </Row>
      </Card>

      <Card>
        <Row
          label="Hotkey behavior"
          hint="Hands-free toggle (press once to start, press again to stop) or push-to-talk (hold)."
          last
        >
          <select
            class="rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s.hands_free_enabled ? "toggle" : "ptt"}
            onChange={(e) =>
              props.update({
                hands_free_enabled: e.currentTarget.value === "toggle",
              })
            }
          >
            <option value="toggle">Hands-free toggle (recommended)</option>
            <option value="ptt">Push-to-talk (hold)</option>
          </select>
        </Row>
      </Card>

      <Card>
        <Row
          label="Language"
          hint="Whisper language hint. Auto detects from speech."
          last
        >
          <select
            class="rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s.language}
            onChange={(e) => props.update({ language: e.currentTarget.value })}
          >
            <For each={LANGUAGE_OPTIONS}>
              {(o) => <option value={o.value}>{o.label}</option>}
            </For>
          </select>
        </Row>
      </Card>

      <Card>
        <Row
          label="Default microphone"
          hint="Vox uses the current system default input device."
          last
        >
          <span class="text-[12px] text-[var(--color-ink-mute)]">
            System default
          </span>
        </Row>
      </Card>
    </>
  );
}

function HotkeysTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [diags, setDiags] = createSignal<Map<string, HotkeyDiagnostic>>(
    new Map(),
  );
  const [testStatus, setTestStatus] = createSignal<string | null>(null);

  async function refreshDiagnostics() {
    try {
      const r = await diagnoseHotkeys();
      const map = new Map<string, HotkeyDiagnostic>();
      for (const e of r.entries) map.set(e.purpose, e);
      setDiags(map);
    } catch (e) {
      console.warn("diagnoseHotkeys failed:", e);
    }
  }

  let unlistenSettings: (() => void) | null = null;

  onMount(() => {
    void refreshDiagnostics();
    // The backend re-registers hotkeys whenever settings are saved
    // (see `commands::set_settings`). Listen for that broadcast and
    // refetch the diagnostic snapshot so the picker rows immediately
    // reflect what the OS actually has bound — including any fallback
    // that kicked in or any "Not bound" state if registration failed.
    void (async () => {
      unlistenSettings = await onSettingsChanged(() => {
        void refreshDiagnostics();
      });
    })();
  });

  onCleanup(() => {
    unlistenSettings?.();
  });

  async function onTest() {
    setTestStatus("Reading…");
    try {
      const a = await getActiveHotkey();
      if (!a.bound) {
        setTestStatus(`Failed: ${a.error ?? "no fallback succeeded"}`);
      } else if (a.fell_back) {
        setTestStatus(`Bound to ${a.bound} (fallback from ${a.configured})`);
      } else {
        setTestStatus(`Bound to ${a.bound}`);
      }
    } catch (e) {
      setTestStatus(`Failed: ${String(e)}`);
    }
  }

  const ptt = () => diags().get("push-to-talk") ?? null;
  const cancel = () => diags().get("cancel") ?? null;
  const cycle = () => diags().get("mode-cycle") ?? null;

  return (
    <>
      <Card>
        <div class="flex items-start justify-between gap-4 border-b border-[var(--color-border)] px-4 py-3">
          <div class="flex flex-col">
            <div class="flex items-center gap-2">
              <span class="text-[13px] text-[var(--color-ink)]">
                Push-to-talk
              </span>
              <span class="rounded-full bg-white/8 px-2 py-0.5 text-[10px] text-[var(--color-ink-mute)]">
                ⌘ + ⌃ + Space
              </span>
            </div>
            <span class="mt-0.5 text-[11px] text-[var(--color-ink-mute)]">
              {pttHint(props.s.hands_free_enabled, ptt())}
            </span>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <button
              type="button"
              onClick={() => void onTest()}
              class="rounded-[6px] bg-white/8 px-3 py-1 text-[11px] hover:bg-white/12"
              title="Read the current binding from the runtime"
            >
              Test
            </button>
            <Show when={testStatus()}>
              <span
                class="rounded-full bg-black/40 px-2 py-0.5 text-[10px]"
                classList={{
                  "text-[var(--color-success)]":
                    testStatus()?.startsWith("Bound") ?? false,
                  "text-[var(--color-danger)]":
                    testStatus()?.startsWith("Failed") ?? false,
                  "text-[var(--color-ink-mute)]":
                    testStatus() === "Reading…",
                }}
              >
                {testStatus()}
              </span>
            </Show>
          </div>
        </div>
        <HotkeyRow
          label="Cancel while recording"
          hint="Abort the pipeline without copying."
          configured={props.s.cancel_hotkey}
          diagnostic={cancel()}
          options={CANCEL_HOTKEY_OPTIONS}
          onChange={(v) => props.update({ cancel_hotkey: v })}
        />
        <HotkeyRow
          label="Cycle voice mode"
          hint="Press to advance through Voice Modes in display order."
          configured={props.s.mode_cycle_hotkey}
          diagnostic={cycle()}
          options={MODE_CYCLE_HOTKEY_OPTIONS}
          onChange={(v) => props.update({ mode_cycle_hotkey: v })}
          last
        />
      </Card>
      <p class="text-[11px] text-[var(--color-ink-mute)]">
        Hotkey changes take effect after the app restarts. If the
        configured key fails to bind on macOS, Vox falls back to the
        next compatible shortcut and shows it under the picker.
      </p>
    </>
  );
}

function HotkeyRow(props: {
  label: string;
  hint: string;
  configured: string;
  diagnostic: HotkeyDiagnostic | null;
  options: HotkeyOption[];
  onChange: (v: string) => void;
  extra?: JSX.Element;
  last?: boolean;
}) {
  const boundLabel = () => {
    const d = props.diagnostic;
    if (!d) return null;
    if (!d.bound) return null;
    return labelForValue(props.options, d.bound) ?? d.bound;
  };
  const configuredLabel = () =>
    labelForValue(props.options, props.configured) ?? props.configured;
  const showBoundOverride = () => {
    const d = props.diagnostic;
    return d && d.fell_back && d.bound && d.bound !== props.configured;
  };
  const failed = () => {
    const d = props.diagnostic;
    return d != null && !d.bound && props.configured !== "";
  };
  const chordRequiresAx = () => {
    const d = props.diagnostic;
    if (!d || d.bound) return false;
    // Heuristic: any failed binding whose error mentions Accessibility
    // OR whose configured value parses as a chord (only modifier
    // tokens). Keeps the link visible even if the underlying error
    // string changes wording — modifier-only specs only ever fail for
    // permission reasons.
    if (d.error?.toLowerCase().includes("accessibility")) return true;
    return isModifierOnlySpec(d.configured);
  };
  const failedChip = () =>
    chordRequiresAx() ? "Not bound — Accessibility required" : "Not bound";
  return (
    <div
      class="flex items-start justify-between gap-4 px-4 py-3"
      classList={{
        "border-b border-[var(--color-border)]": !props.last,
      }}
    >
      <div class="flex flex-col">
        <div class="flex items-center gap-2">
          <span class="text-[13px] text-[var(--color-ink)]">{props.label}</span>
          <Show when={showBoundOverride()}>
            <span class="rounded-full bg-[var(--color-accent)]/20 px-2 py-0.5 text-[10px] text-[var(--color-accent)]">
              Bound to {boundLabel()}
            </span>
          </Show>
          <Show when={failed()}>
            <span class="rounded-full bg-[var(--color-danger)]/20 px-2 py-0.5 text-[10px] text-[var(--color-danger)]">
              {failedChip()}
            </span>
          </Show>
        </div>
        <span class="text-[11px] text-[var(--color-ink-mute)]">
          {props.hint}
        </span>
        <Show when={showBoundOverride()}>
          <span class="text-[11px] text-[var(--color-ink-faint)]">
            Configured {configuredLabel()} couldn't be bound on this
            Mac — Vox is using {boundLabel()}.
          </span>
        </Show>
        <Show when={failed() && props.diagnostic?.error}>
          <span class="text-[11px] text-[var(--color-danger)]">
            {props.diagnostic?.error}
          </span>
        </Show>
        <Show when={chordRequiresAx()}>
          <button
            type="button"
            onClick={() => void openAccessibilitySettings()}
            class="mt-1 self-start rounded-[6px] bg-[var(--color-danger)]/15 px-2 py-1 text-[11px] text-[var(--color-danger)] hover:bg-[var(--color-danger)]/25"
          >
            Open Accessibility settings
          </button>
        </Show>
      </div>
      <div class="flex shrink-0 items-center gap-2">
        <select
          class="rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
          value={props.configured}
          onChange={(e) => props.onChange(e.currentTarget.value)}
        >
          <For each={props.options}>
            {(o) => (
              <option value={o.value}>
                {o.warn ? "⚠︎ " : ""}
                {o.label}
              </option>
            )}
          </For>
        </select>
        {props.extra}
      </div>
    </div>
  );
}

function labelForValue(opts: HotkeyOption[], value: string): string | null {
  const m = opts.find((o) => o.value === value);
  return m?.label ?? null;
}

/** True if the hotkey id is composed only of modifier tokens (no base
 * key). Mirrors the parser in `mac_chord::try_parse_chord`. */
function isModifierOnlySpec(spec: string | null | undefined): boolean {
  if (!spec) return false;
  const tokens = spec.split("+").map((t) => t.trim());
  if (tokens.length === 0 || tokens.some((t) => !t)) return false;
  const MODS = new Set([
    "Alt",
    "Option",
    "Ctrl",
    "Control",
    "Cmd",
    "Command",
    "Meta",
    "Super",
    "CommandOrControl",
    "Shift",
  ]);
  return tokens.every((t) => MODS.has(t));
}

/** Subtext under the push-to-talk row. When the bound hotkey is a
 * modifier-only chord we print "Hold both keys to dictate. Release
 * either to stop." — otherwise the standard single-key copy. */
function pttHint(
  handsFree: boolean,
  diag: HotkeyDiagnostic | null,
): string {
  if (handsFree) {
    return "Press once to start recording, press again to stop. (Hands-free is on.)";
  }
  if (diag?.mechanism === "chord") {
    return "Hold both keys to dictate. Release either to stop.";
  }
  return "Hold the key to dictate. Release to stop.";
}

// -------- Models --------

function ModelsTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [models, setModels] = createSignal<ModelStatus[]>([]);
  const [progress, setProgress] = createSignal<
    Record<string, { downloaded: number; total: number; eta: number | null }>
  >({});
  const [downloading, setDownloading] = createSignal<Set<string>>(new Set());

  async function refresh() {
    try {
      setModels(await listModels());
    } catch (e) {
      console.error("listModels:", e);
    }
  }

  let unlistens: (() => void)[] = [];
  onMount(async () => {
    await refresh();
    unlistens.push(
      await onModelProgress((p) => {
        setProgress((prev) => ({
          ...prev,
          [p.id]: {
            downloaded: p.bytes_downloaded,
            total: p.total_bytes,
            eta: p.eta_secs,
          },
        }));
      }),
    );
    unlistens.push(
      await onModelDone(async (p) => {
        setDownloading((prev) => {
          const next = new Set(prev);
          next.delete(p.id);
          return next;
        });
        setProgress((prev) => {
          const { [p.id]: _, ...rest } = prev;
          return rest;
        });
        await refresh();
      }),
    );
    unlistens.push(
      await onModelError((p) => {
        console.error("model error", p);
        setDownloading((prev) => {
          const next = new Set(prev);
          next.delete(p.id);
          return next;
        });
      }),
    );
    unlistens.push(
      await onModelCanceled((p) => {
        setDownloading((prev) => {
          const next = new Set(prev);
          next.delete(p.id);
          return next;
        });
        setProgress((prev) => {
          const { [p.id]: _, ...rest } = prev;
          return rest;
        });
      }),
    );
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
  });

  async function startDownload(id: string) {
    setDownloading((prev) => {
      const next = new Set(prev);
      next.add(id);
      return next;
    });
    try {
      await downloadModel(id);
    } catch (e) {
      console.error("download:", e);
      setDownloading((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }

  async function cancel(id: string) {
    await cancelDownload(id);
  }

  async function remove(id: string) {
    if (!confirm(`Delete the cached model ${id}?`)) return;
    try {
      await deleteModel(id);
    } catch (e) {
      console.error("delete:", e);
    }
    await refresh();
  }

  function setActiveStt(modelId: string) {
    // Only one STT engine ships now (whisper-local). Keep the field in
    // sync but never offer to flip it elsewhere.
    props.update({
      stt_engine: "whisper-local",
      stt_model_id: modelId,
    });
  }

  function setActiveLlm(modelId: string) {
    // Local-only LLM cleanup. The model selector is purely for choosing
    // which Gemma weights live on disk; the engine id is fixed.
    props.update({
      llm_engine: "gemma-local",
      llm_model_id: modelId,
    });
  }

  const sttModels = createMemo(() =>
    models().filter((m) => m.descriptor.kind === "stt"),
  );
  const llmModels = createMemo(() =>
    models().filter((m) => m.descriptor.kind === "llm"),
  );

  // The on/off switch the user sees. Internally it picks between
  // `gemma-local` (when on) and `none` (when off). The `llm_engine`
  // field stays in lockstep so the rest of the app keeps a single
  // source of truth, while `polishing_enabled` carries the user's
  // *intent* across periods when the local LLM is unavailable (so we
  // can restore Gemma the moment it stabilises).
  function setPolishing(on: boolean) {
    props.update({
      polishing_enabled: on,
      llm_engine: on ? "gemma-local" : "none",
    });
  }

  return (
    <>
      <Card>
        <Row
          label="Transcription"
          hint="Whisper runs entirely on this Mac. No audio ever leaves the device."
          last
        >
          <span class="text-[12px] text-[var(--color-ink-mute)]">
            Local (Whisper)
          </span>
        </Row>
      </Card>
      <div class="mb-4 text-[11px] uppercase tracking-wider text-[var(--color-ink-faint)]">
        Whisper models
      </div>
      <For each={sttModels()}>
        {(m) => (
          <ModelRow
            m={m}
            isActive={props.s.stt_model_id === m.descriptor.id}
            progress={progress()[m.descriptor.id]}
            downloading={downloading().has(m.descriptor.id)}
            onUse={() => setActiveStt(m.descriptor.id)}
            onDownload={() => startDownload(m.descriptor.id)}
            onCancel={() => cancel(m.descriptor.id)}
            onDelete={() => remove(m.descriptor.id)}
          />
        )}
      </For>

      <Card>
        <Row
          label="Polish transcripts"
          hint="Run a local Gemma model on each transcript to fix punctuation, remove filler words, and apply your active Voice Mode. When off, raw output is copied verbatim."
          last
        >
          <Toggle
            checked={props.s.polishing_enabled}
            onChange={(v) => setPolishing(v)}
          />
        </Row>
      </Card>
      <Show when={props.s.polishing_enabled}>
        <Show when={!props.s.gemma_tos_accepted}>
          <div class="mb-3 rounded-[10px] bg-[rgba(124,156,255,0.08)] p-3 ring-1 ring-[var(--color-accent)]/40 text-[12px] text-[var(--color-ink)]">
            You'll be asked to accept the{" "}
            <a
              class="underline"
              href="https://ai.google.dev/gemma/terms"
              target="_blank"
            >
              Gemma Terms of Use
            </a>{" "}
            on first download.
          </div>
        </Show>
        <div class="mb-4 text-[11px] uppercase tracking-wider text-[var(--color-ink-faint)]">
          Cleanup models
        </div>
        <For each={llmModels()}>
          {(m) => (
            <ModelRow
              m={m}
              isActive={props.s.llm_model_id === m.descriptor.id}
              progress={progress()[m.descriptor.id]}
              downloading={downloading().has(m.descriptor.id)}
              onUse={() => setActiveLlm(m.descriptor.id)}
              onDownload={() => startDownload(m.descriptor.id)}
              onCancel={() => cancel(m.descriptor.id)}
              onDelete={() => remove(m.descriptor.id)}
            />
          )}
        </For>
      </Show>
    </>
  );
}

function ModelRow(props: {
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

  function formatBytes(b: number) {
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(0)} KB`;
    if (b < 1024 * 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MB`;
    return `${(b / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  return (
    <div class="mb-3 rounded-[10px] bg-[var(--color-surface-raised)] p-4 ring-1 ring-[var(--color-border)]">
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
          <div class="mt-0.5 text-[11px] text-[var(--color-ink-mute)]">
            ~{sizeMb()} MB on disk · ~{ramMb()} MB peak RAM · tier:{" "}
            {props.m.descriptor.tier}
          </div>
        </div>
        <div class="flex items-center gap-2">
          <Show when={props.m.installed && !props.isActive}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)]/80 px-3 py-1.5 text-[12px] font-medium text-black hover:bg-[var(--color-accent)]"
              onClick={props.onUse}
            >
              Use
            </button>
          </Show>
          <Show when={!props.m.installed && !props.downloading}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1.5 text-[12px] font-medium text-black hover:brightness-110"
              onClick={props.onDownload}
            >
              Download
            </button>
          </Show>
          <Show when={props.downloading}>
            <button
              type="button"
              class="rounded-[6px] bg-white/8 px-3 py-1.5 text-[12px] hover:bg-white/12"
              onClick={props.onCancel}
            >
              Cancel
            </button>
          </Show>
          <Show when={props.m.installed}>
            <button
              type="button"
              class="rounded-[6px] bg-white/6 px-3 py-1.5 text-[12px] text-[var(--color-ink-mute)] hover:bg-[var(--color-danger)]/20 hover:text-[var(--color-danger)]"
              onClick={props.onDelete}
              title="Delete cached model"
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
            <div class="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-white/8">
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
    </div>
  );
}

// -------- Modes --------

function ModesTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [modes, setModes] = createSignal<VoiceMode[]>([]);
  const [editing, setEditing] = createSignal<VoiceMode | null>(null);
  const [importErr, setImportErr] = createSignal<string | null>(null);
  let fileInput: HTMLInputElement | undefined;

  async function refresh() {
    try {
      setModes(await listModes());
    } catch (e) {
      console.error("listModes:", e);
    }
  }

  onMount(async () => {
    await refresh();
  });

  // Drag-drop file import on the whole modes tab.
  function onDragOver(e: DragEvent) {
    e.preventDefault();
  }
  async function onDrop(e: DragEvent) {
    e.preventDefault();
    const file = e.dataTransfer?.files?.[0];
    if (!file) return;
    if (!file.name.endsWith(".voxmode") && !file.name.endsWith(".json")) {
      setImportErr("Only .voxmode or .json files are supported.");
      return;
    }
    const text = await file.text();
    await doImport(text);
  }

  async function doImport(text: string) {
    setImportErr(null);
    try {
      await importModeJson(text);
      await refresh();
    } catch (e) {
      setImportErr(String(e));
    }
  }

  async function onFilePick(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    const text = await file.text();
    input.value = "";
    await doImport(text);
  }

  async function onActivate(id: string) {
    props.update({ active_mode_id: id });
  }

  async function onDuplicate(id: string) {
    try {
      const m = await duplicateMode(id);
      await refresh();
      setEditing(m);
    } catch (e) {
      console.error("duplicate:", e);
    }
  }

  async function onDelete(id: string) {
    if (!confirm("Delete this mode? This cannot be undone.")) return;
    try {
      await deleteMode(id);
      await refresh();
      if (editing()?.id === id) setEditing(null);
    } catch (e) {
      console.error("delete:", e);
    }
  }

  async function onExport(m: VoiceMode) {
    try {
      const json = await exportMode(m.id);
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${m.id}.voxmode`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 0);
    } catch (e) {
      console.error("export:", e);
    }
  }

  async function onReset() {
    try {
      const n = await resetDefaultModes();
      await refresh();
      if (n === 0) alert("All defaults are already present.");
    } catch (e) {
      console.error("reset:", e);
    }
  }

  function onNew() {
    const draft: VoiceMode = {
      id: "",
      name: "New Mode",
      icon: "🌟",
      description: "",
      system_prompt:
        "You clean up dictated text. Return plain text only — no preface.",
      dictionary: [],
      punctuation: "Standard",
      case: "Sentence",
      max_length_chars: null,
      auto_triggers: [],
      built_in: false,
      created_at: 0,
      updated_at: 0,
    };
    setEditing(draft);
  }

  async function onSaveDraft(mode: VoiceMode) {
    let id = mode.id;
    if (!id) {
      // Generate a slug client-side; the backend will dedup if it
      // collides with an existing mode.
      id = mode.name
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/(^-|-$)/g, "");
      if (!id) id = `mode-${Date.now()}`;
    }
    const toSave: VoiceMode = { ...mode, id };
    try {
      await saveMode(toSave);
      await refresh();
      setEditing(null);
    } catch (e) {
      console.error("save mode:", e);
      alert(`Save failed: ${e}`);
    }
  }

  return (
    <div onDragOver={onDragOver} onDrop={onDrop}>
      <Card>
        <Row
          label="Active mode"
          hint="Used for the next utterance unless you cycle or pick a different mode."
        >
          <select
            class="rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s.active_mode_id}
            onChange={(e) => onActivate(e.currentTarget.value)}
          >
            <For each={modes()}>
              {(m) => (
                <option value={m.id}>
                  {m.icon} {m.name}
                </option>
              )}
            </For>
          </select>
        </Row>
        <Row
          label="Show suggestion HUD"
          hint="Hint a mode based on the focused app. Vox never auto-switches."
          last
        >
          <Toggle
            checked={props.s.auto_suggest_modes}
            onChange={(v) => props.update({ auto_suggest_modes: v })}
          />
        </Row>
      </Card>

      <Show when={importErr()}>
        <div class="mb-3 rounded-[10px] bg-[var(--color-danger)]/10 p-3 text-[12px] text-[var(--color-danger)] ring-1 ring-[var(--color-danger)]/40">
          Import failed: {importErr()}
        </div>
      </Show>

      <For each={modes()}>
        {(m) => (
          <ModeRow
            m={m}
            isActive={m.id === props.s.active_mode_id}
            onActivate={() => onActivate(m.id)}
            onEdit={() => setEditing(m)}
            onDuplicate={() => onDuplicate(m.id)}
            onDelete={() => onDelete(m.id)}
            onExport={() => onExport(m)}
          />
        )}
      </For>

      <div class="mt-2 flex flex-wrap gap-2">
        <button
          type="button"
          onClick={onNew}
          class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1.5 text-[12px] font-medium text-black hover:brightness-110"
        >
          + New mode
        </button>
        <button
          type="button"
          onClick={() => fileInput?.click()}
          class="rounded-[6px] bg-white/8 px-3 py-1.5 text-[12px] hover:bg-white/12"
        >
          Import .voxmode
        </button>
        <button
          type="button"
          onClick={onReset}
          class="rounded-[6px] bg-white/8 px-3 py-1.5 text-[12px] hover:bg-white/12"
        >
          Reset to defaults
        </button>
        <input
          ref={fileInput}
          type="file"
          accept=".voxmode,.json,application/json"
          class="hidden"
          onChange={onFilePick}
        />
      </div>
      <p class="mt-3 text-[11px] text-[var(--color-ink-mute)]">
        Drag a <code class="text-[var(--color-ink)]">.voxmode</code> file
        anywhere on this tab to import. Built-in modes can't be edited
        directly — duplicate one to make changes.
      </p>

      <Show when={editing()}>
        {(m) => (
          <ModeEditor
            mode={m()}
            onCancel={() => setEditing(null)}
            onSave={onSaveDraft}
          />
        )}
      </Show>
    </div>
  );
}

function ModeRow(props: {
  m: VoiceMode;
  isActive: boolean;
  onActivate: () => void;
  onEdit: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onExport: () => void;
}) {
  return (
    <div class="mb-3 rounded-[10px] bg-[var(--color-surface-raised)] p-4 ring-1 ring-[var(--color-border)]">
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1">
          <div class="flex items-center gap-2">
            <span class="text-[16px]" aria-hidden="true">
              {props.m.icon}
            </span>
            <span class="text-[13px] font-medium">{props.m.name}</span>
            <Show when={props.m.built_in}>
              <span class="rounded-full bg-white/8 px-2 py-0.5 text-[10px] text-[var(--color-ink-mute)]">
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
          <div class="mt-0.5 text-[11px] text-[var(--color-ink-mute)]">
            {props.m.description || "No description."}
          </div>
        </div>
        <div class="flex shrink-0 flex-wrap items-center gap-1.5">
          <Show when={!props.isActive}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)]/80 px-3 py-1.5 text-[12px] font-medium text-black hover:bg-[var(--color-accent)]"
              onClick={props.onActivate}
            >
              Use
            </button>
          </Show>
          <Show when={props.m.built_in}>
            <button
              type="button"
              class="rounded-[6px] bg-white/8 px-3 py-1.5 text-[12px] hover:bg-white/12"
              onClick={props.onDuplicate}
            >
              Duplicate to edit
            </button>
          </Show>
          <Show when={!props.m.built_in}>
            <button
              type="button"
              class="rounded-[6px] bg-white/8 px-3 py-1.5 text-[12px] hover:bg-white/12"
              onClick={props.onEdit}
            >
              Edit
            </button>
            <button
              type="button"
              class="rounded-[6px] bg-white/6 px-2 py-1.5 text-[12px] text-[var(--color-ink-mute)] hover:bg-white/10"
              onClick={props.onExport}
              title="Export as .voxmode"
            >
              Export
            </button>
            <button
              type="button"
              class="rounded-[6px] bg-white/6 px-2 py-1.5 text-[12px] text-[var(--color-ink-mute)] hover:bg-[var(--color-danger)]/20 hover:text-[var(--color-danger)]"
              onClick={props.onDelete}
            >
              Delete
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}

function ModeEditor(props: {
  mode: VoiceMode;
  onCancel: () => void;
  onSave: (m: VoiceMode) => void;
}) {
  const [draft, setDraft] = createSignal<VoiceMode>({ ...props.mode });
  const [dictDraft, setDictDraft] = createSignal("");

  function patch(partial: Partial<VoiceMode>) {
    setDraft({ ...draft(), ...partial });
  }

  function addDictTerm() {
    const v = dictDraft().trim();
    if (!v) return;
    if (draft().dictionary.includes(v)) {
      setDictDraft("");
      return;
    }
    patch({ dictionary: [...draft().dictionary, v] });
    setDictDraft("");
  }

  function removeDictTerm(t: string) {
    patch({ dictionary: draft().dictionary.filter((d) => d !== t) });
  }

  function addTrigger() {
    patch({
      auto_triggers: [
        ...draft().auto_triggers,
        { bundle_id_glob: "", window_title_substring: null },
      ],
    });
  }

  function updateTrigger(idx: number, field: "bundle_id_glob" | "window_title_substring", value: string) {
    const triggers = draft().auto_triggers.map((t, i) => {
      if (i !== idx) return t;
      const next = { ...t };
      next[field] = value || null;
      return next;
    });
    patch({ auto_triggers: triggers });
  }

  function removeTrigger(idx: number) {
    patch({
      auto_triggers: draft().auto_triggers.filter((_, i) => i !== idx),
    });
  }

  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 px-6 py-8">
      <div class="flex h-full w-full max-w-2xl flex-col overflow-hidden rounded-[12px] bg-[var(--color-bg)] ring-1 ring-[var(--color-border)]">
        <header class="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
          <h2 class="text-[14px] font-semibold">
            {props.mode.id ? "Edit mode" : "New mode"}
          </h2>
          <button
            type="button"
            class="rounded p-1 text-[var(--color-ink-mute)] hover:bg-white/8 hover:text-[var(--color-ink)]"
            onClick={props.onCancel}
            aria-label="Cancel"
          >
            ×
          </button>
        </header>
        <div class="flex-1 overflow-y-auto scrollbar-thin px-4 py-4">
          <Card>
            <Row label="Name">
              <input
                type="text"
                class="w-48 rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().name}
                onInput={(e) => patch({ name: e.currentTarget.value })}
              />
            </Row>
            <Row label="Icon (emoji or text)">
              <input
                type="text"
                class="w-24 rounded-[6px] bg-black/30 px-2 py-1 text-center text-[14px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().icon}
                onInput={(e) => patch({ icon: e.currentTarget.value })}
              />
            </Row>
            <Row label="Description" last>
              <input
                type="text"
                class="w-64 rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().description}
                onInput={(e) => patch({ description: e.currentTarget.value })}
              />
            </Row>
          </Card>

          <Card>
            <div class="px-4 py-3">
              <div class="mb-1 text-[12px] font-medium">System prompt</div>
              <textarea
                class="h-48 w-full rounded-[6px] bg-black/30 px-2 py-2 font-mono text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().system_prompt}
                onInput={(e) => patch({ system_prompt: e.currentTarget.value })}
              />
              <p class="mt-1 text-[11px] text-[var(--color-ink-mute)]">
                {draft().system_prompt.length} chars. Tip: keep it tight
                (~150–250 tokens) and end with explicit "Output only the
                cleaned text."
              </p>
            </div>
          </Card>

          <Card>
            <Row label="Punctuation">
              <select
                class="rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().punctuation}
                onChange={(e) =>
                  patch({
                    punctuation: e.currentTarget.value as PunctuationStyle,
                  })
                }
              >
                <For each={PUNCTUATION_OPTIONS}>
                  {(o) => <option value={o.value}>{o.label}</option>}
                </For>
              </select>
            </Row>
            <Row label="Case">
              <select
                class="rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().case}
                onChange={(e) =>
                  patch({ case: e.currentTarget.value as CaseStyle })
                }
              >
                <For each={CASE_OPTIONS}>
                  {(o) => <option value={o.value}>{o.label}</option>}
                </For>
              </select>
            </Row>
            <Row
              label="Max length"
              hint="0 or empty = no cap. Truncates at the last sentence boundary that fits."
              last
            >
              <input
                type="number"
                min="0"
                class="w-20 rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().max_length_chars ?? ""}
                onInput={(e) => {
                  const v = e.currentTarget.value.trim();
                  patch({ max_length_chars: v === "" ? null : Number(v) });
                }}
              />
            </Row>
          </Card>

          <Card>
            <div class="px-4 py-3">
              <div class="mb-2 text-[12px] font-medium">
                Mode dictionary
              </div>
              <p class="mb-2 text-[11px] text-[var(--color-ink-mute)]">
                Appended to the global dictionary at runtime.
              </p>
              <div class="mb-2 flex gap-2">
                <input
                  type="text"
                  class="flex-1 rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                  placeholder="Add proper nouns, acronyms, or technical terms — one per line."
                  value={dictDraft()}
                  onInput={(e) => setDictDraft(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      addDictTerm();
                    }
                  }}
                />
                <button
                  type="button"
                  class="rounded-[6px] bg-white/8 px-3 py-1 text-[12px] hover:bg-white/12"
                  onClick={addDictTerm}
                >
                  Add
                </button>
              </div>
              <Show when={draft().dictionary.length > 0}>
                <div class="flex flex-wrap gap-2">
                  <For each={draft().dictionary}>
                    {(term) => (
                      <button
                        type="button"
                        onClick={() => removeDictTerm(term)}
                        class="group inline-flex items-center gap-1 rounded-[6px] bg-white/6 px-2 py-1 text-[12px] hover:bg-white/10"
                      >
                        <span>{term}</span>
                        <span class="text-[var(--color-ink-mute)] group-hover:text-[var(--color-danger)]">
                          ×
                        </span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </Card>

          <Card>
            <div class="px-4 py-3">
              <div class="mb-2 flex items-center justify-between">
                <span class="text-[12px] font-medium">
                  Auto-triggers (suggest only)
                </span>
                <button
                  type="button"
                  class="rounded-[6px] bg-white/8 px-2 py-1 text-[11px] hover:bg-white/12"
                  onClick={addTrigger}
                >
                  + Trigger
                </button>
              </div>
              <p class="mb-2 text-[11px] text-[var(--color-ink-mute)]">
                Vox never auto-switches. These rules feed the suggestion
                HUD on the listening pill. Trailing wildcards allowed
                (e.g. <code>com.jetbrains.*</code>).
              </p>
              <For each={draft().auto_triggers}>
                {(t, i) => (
                  <div class="mb-2 flex items-center gap-2">
                    <input
                      type="text"
                      class="flex-1 rounded-[6px] bg-black/30 px-2 py-1 text-[11px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                      placeholder="bundle id (e.g. com.apple.Notes)"
                      value={t.bundle_id_glob ?? ""}
                      onInput={(e) =>
                        updateTrigger(
                          i(),
                          "bundle_id_glob",
                          e.currentTarget.value,
                        )
                      }
                    />
                    <input
                      type="text"
                      class="flex-1 rounded-[6px] bg-black/30 px-2 py-1 text-[11px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                      placeholder="window title contains (optional)"
                      value={t.window_title_substring ?? ""}
                      onInput={(e) =>
                        updateTrigger(
                          i(),
                          "window_title_substring",
                          e.currentTarget.value,
                        )
                      }
                    />
                    <button
                      type="button"
                      class="rounded-[6px] bg-white/6 px-2 py-1 text-[11px] hover:bg-[var(--color-danger)]/20 hover:text-[var(--color-danger)]"
                      onClick={() => removeTrigger(i())}
                    >
                      ×
                    </button>
                  </div>
                )}
              </For>
            </div>
          </Card>
        </div>
        <footer class="flex items-center justify-end gap-2 border-t border-[var(--color-border)] px-4 py-3">
          <button
            type="button"
            class="rounded-[6px] bg-white/6 px-3 py-1.5 text-[12px] hover:bg-white/10"
            onClick={props.onCancel}
          >
            Cancel
          </button>
          <button
            type="button"
            class="rounded-[6px] bg-[var(--color-accent)] px-4 py-1.5 text-[12px] font-medium text-black hover:brightness-110"
            onClick={() => props.onSave(draft())}
          >
            Save
          </button>
        </footer>
      </div>
    </div>
  );
}

// -------- Dictionary / Privacy / About --------

// 2026-04-26: API Keys tab and ProviderKey component removed — Vox is
// fully on-device, so there are no cloud secrets to enter. If a future
// cloud engine is reintroduced, restore the tab in the `TABS` list and
// rebuild the picker here.

function DictionaryTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [draft, setDraft] = createSignal("");

  function add() {
    const v = draft().trim();
    if (!v) return;
    if (props.s.dictionary.includes(v)) {
      setDraft("");
      return;
    }
    props.update({ dictionary: [...props.s.dictionary, v] });
    setDraft("");
  }

  function remove(term: string) {
    props.update({
      dictionary: props.s.dictionary.filter((t) => t !== term),
    });
  }

  return (
    <>
      <Card>
        <div class="px-4 py-3">
          <p class="mb-3 text-[11px] text-[var(--color-ink-mute)]">
            Names, acronyms, and technical terms Vox should spell correctly.
            Applied during STT bias and the cleanup step.
          </p>
          <div class="mb-3 flex gap-2">
            <input
              type="text"
              class="flex-1 rounded-[6px] bg-black/30 px-3 py-2 text-[13px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
              placeholder="Add proper nouns, acronyms, or technical terms — one per line."
              value={draft()}
              onInput={(e) => setDraft(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  add();
                }
              }}
            />
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)] px-4 py-2 text-[12px] font-medium text-black hover:brightness-110 disabled:opacity-50"
              disabled={!draft().trim()}
              onClick={add}
            >
              Add
            </button>
          </div>
          <Show
            when={props.s.dictionary.length > 0}
            fallback={
              <p class="text-[12px] text-[var(--color-ink-mute)]">
                Add names or technical words Vox should spell correctly.
              </p>
            }
          >
            <div class="flex flex-wrap gap-2">
              <For each={props.s.dictionary}>
                {(term) => (
                  <button
                    type="button"
                    onClick={() => remove(term)}
                    class="group inline-flex items-center gap-1 rounded-[6px] bg-white/6 px-2 py-1 text-[12px] hover:bg-white/10"
                    title="Click to remove"
                  >
                    <span>{term}</span>
                    <span class="text-[var(--color-ink-mute)] group-hover:text-[var(--color-danger)]">
                      ×
                    </span>
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Card>
      <p class="text-[11px] text-[var(--color-ink-mute)]">
        {props.s.dictionary.length} term
        {props.s.dictionary.length === 1 ? "" : "s"}.
      </p>
    </>
  );
}

function PrivacyTab(_props: { s: Settings }) {
  return (
    <>
      <Card>
        <Row label="Audio" hint="Stays on this Mac." last>
          <span class="rounded-full bg-[var(--color-success)]/20 px-2 py-0.5 text-[10px] text-[var(--color-success)]">
            LOCAL
          </span>
        </Row>
      </Card>
      <Card>
        <Row
          label="Transcript polishing"
          hint="Stays on this Mac (or skipped, if polishing is off)."
          last
        >
          <span class="rounded-full bg-[var(--color-success)]/20 px-2 py-0.5 text-[10px] text-[var(--color-success)]">
            LOCAL
          </span>
        </Row>
      </Card>

      <p class="mb-4 text-[11px] text-[var(--color-ink-mute)]">
        Vox is now fully on-device. There is no setting that sends your
        audio or transcripts to any external service.
      </p>

      <div class="mb-5 rounded-[10px] bg-[var(--color-surface-raised)] p-4 ring-1 ring-[var(--color-border)]">
        <h3 class="mb-2 text-[13px] font-medium">What never leaves</h3>
        <ul class="space-y-1 text-[12px] text-[var(--color-ink-mute)]">
          <li>• No audio is ever written to disk.</li>
          <li>• No transcripts are sent over the network.</li>
          <li>• No transcript history is persisted across app quits.</li>
          <li>• No telemetry, no analytics, no crash uploads.</li>
          <li>• No API keys are stored — there is nothing to leak.</li>
        </ul>
      </div>
    </>
  );
}

function AboutTab() {
  const [checking, setChecking] = createSignal(false);
  const [updateResult, setUpdateResult] = createSignal<UpdateCheckResult | null>(
    null,
  );
  const [diag, setDiag] = createSignal<DiagnoseHotkeys | null>(null);
  const [diagErr, setDiagErr] = createSignal<string | null>(null);
  const [devInfo, setDevInfo] = createSignal<DevBinaryInfo | null>(null);
  const [devCopied, setDevCopied] = createSignal(false);

  async function onReplay() {
    try {
      await replayOnboarding();
    } catch (err) {
      console.error(err);
    }
  }

  async function onDiagnostics() {
    setDiagErr(null);
    setDiag(null);
    try {
      setDiag(await diagnoseHotkeys());
    } catch (err) {
      setDiagErr(String(err));
    }
  }

  async function onRevealDevPath() {
    try {
      const info = await revealDevBinaryPath();
      setDevInfo(info);
      // Best-effort: copy the dev binary path to the clipboard so the
      // user can paste it into System Settings → Privacy & Security →
      // Accessibility's Cmd+Shift+G "Go to folder" prompt.
      try {
        await navigator.clipboard.writeText(info.binary_path);
        setDevCopied(true);
      } catch {
        setDevCopied(false);
      }
      await openAccessibilitySettings();
    } catch (err) {
      console.error(err);
    }
  }

  async function onCheckUpdates() {
    setChecking(true);
    setUpdateResult(null);
    try {
      const r = await checkForUpdates();
      setUpdateResult(r);
    } catch (err) {
      setUpdateResult({
        status: "error",
        message: String(err),
      });
    } finally {
      setChecking(false);
    }
  }

  return (
    <>
      <div class="mb-5 rounded-[10px] bg-[var(--color-surface-raised)] p-5 ring-1 ring-[var(--color-border)]">
        <div class="mb-1 text-[20px] font-semibold">Vox</div>
        <div class="text-[13px] text-[var(--color-ink-mute)]">
          Talk to any app — now offline by default.
        </div>
        <div class="mt-3 text-[11px] text-[var(--color-ink-faint)]">
          Version 1.0.0 · Local-first · Built with Tauri
        </div>
      </div>
      <Card>
        <Row
          label="Check for updates"
          hint="Auto-update requires a signed release."
        >
          <button
            type="button"
            onClick={onCheckUpdates}
            disabled={checking()}
            class="rounded-[6px] bg-white/6 px-3 py-1.5 text-[12px] hover:bg-white/10 disabled:opacity-40"
          >
            {checking() ? "Checking…" : "Check now"}
          </button>
        </Row>
        <Show when={updateResult()}>
          {(r) => (
            <div class="px-4 py-3 border-b border-[var(--color-border)]">
              <span
                class="text-[12px]"
                classList={{
                  "text-[var(--color-success)]": r().status === "available",
                  "text-[var(--color-ink-mute)]":
                    r().status === "up-to-date" ||
                    r().status === "unconfigured",
                  "text-[var(--color-danger)]": r().status === "error",
                }}
              >
                {r().message ?? r().status}
              </span>
            </div>
          )}
        </Show>
        <Row
          label="Replay onboarding"
          hint="Walk through the setup wizard again."
        >
          <button
            type="button"
            onClick={onReplay}
            class="rounded-[6px] bg-white/6 px-3 py-1.5 text-[12px] hover:bg-white/10"
          >
            Start
          </button>
        </Row>
        <Row
          label="Diagnostics"
          hint="Print hotkey bindings and any registration errors."
        >
          <button
            type="button"
            onClick={() => void onDiagnostics()}
            class="rounded-[6px] bg-white/6 px-3 py-1.5 text-[12px] hover:bg-white/10"
          >
            Run
          </button>
        </Row>
        <Row
          label="Open Accessibility settings"
          hint="Copies this build's binary path to the clipboard so you can paste it into Privacy & Security → Accessibility (Cmd+Shift+G)."
        >
          <button
            type="button"
            onClick={() => void onRevealDevPath()}
            class="rounded-[6px] bg-white/6 px-3 py-1.5 text-[12px] hover:bg-white/10"
          >
            Open & copy path
          </button>
        </Row>
        <Show when={devInfo()}>
          {(d) => (
            <div class="px-4 py-3 border-b border-[var(--color-border)] flex flex-col gap-1">
              <span class="text-[11px] text-[var(--color-ink-mute)]">
                Bundle id: <code class="text-[var(--color-ink)]">{d().bundle_identifier}</code>
                <Show when={d().is_dev_build}>
                  <span class="ml-2 rounded-full bg-[var(--color-accent)]/20 px-2 py-0.5 text-[10px] text-[var(--color-accent)]">dev build</span>
                </Show>
                <Show when={!d().accessibility_granted}>
                  <span class="ml-2 rounded-full bg-[var(--color-danger)]/20 px-2 py-0.5 text-[10px] text-[var(--color-danger)]">accessibility denied</span>
                </Show>
              </span>
              <span class="text-[11px] text-[var(--color-ink-faint)] break-all">
                {d().binary_path}
              </span>
              <Show when={devCopied()}>
                <span class="text-[11px] text-[var(--color-success)]">
                  Path copied to clipboard.
                </span>
              </Show>
            </div>
          )}
        </Show>
        <Show when={diagErr()}>
          <div class="px-4 py-3 text-[12px] text-[var(--color-danger)] border-b border-[var(--color-border)]">
            {diagErr()}
          </div>
        </Show>
        <Show when={diag()}>
          {(d) => (
            <div class="px-4 py-3 border-b border-[var(--color-border)]">
              <pre class="overflow-x-auto rounded-[6px] bg-black/40 p-3 text-[11px] text-[var(--color-ink)]">
                {formatDiagnostics(d())}
              </pre>
            </div>
          )}
        </Show>
        <Row
          label="GitHub"
          hint="Report an issue or read the source."
          last
        >
          <span class="text-[12px] text-[var(--color-ink-mute)]">
            (coming soon)
          </span>
        </Row>
      </Card>
    </>
  );
}

function formatDiagnostics(d: DiagnoseHotkeys): string {
  if (d.entries.length === 0) {
    return "No hotkey state available — restart the app.";
  }
  const lines: string[] = [];
  for (const e of d.entries) {
    const cfg = e.configured || "<off>";
    const bnd = e.bound || "<unbound>";
    const mech = e.mechanism ? ` mechanism=${e.mechanism}` : "";
    lines.push(
      `${e.purpose.padEnd(14)} configured=${cfg.padEnd(28)} bound=${bnd}${mech}` +
      (e.fell_back ? "  [fallback]" : ""),
    );
    if (e.error_chain.length > 0) {
      for (const err of e.error_chain) {
        lines.push(`               · ${err}`);
      }
    }
  }
  return lines.join("\n");
}
