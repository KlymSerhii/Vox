import {
  createMemo,
  createResource,
  createSignal,
  For,
  type JSX,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import {
  appleFmAvailability,
  cancelDownload,
  cancelInferenceServerDownload,
  checkForUpdates,
  deleteInferenceServer,
  deleteMode,
  deleteModel,
  diagnoseHotkeys,
  downloadInferenceServer,
  downloadModel,
  duplicateMode,
  exportMode,
  getActiveHotkey,
  getAppVersion,
  getLaunchAtLogin,
  getSettings,
  importModeJson,
  inferenceServerStatus,
  languageDisplayName,
  listModels,
  listModes,
  onBinaryCanceled,
  onBinaryDone,
  onBinaryError,
  onBinaryProgress,
  onModelCanceled,
  onModelDone,
  onModelError,
  onModelProgress,
  onSettingsChanged,
  openAccessibilitySettings,
  PARAKEET_SUPPORTED_LANGUAGES,
  parakeetAvailability,
  parakeetSupportsLanguage,
  replayOnboarding,
  resetDefaultModes,
  restartInferenceServer,
  revealDevBinaryPath,
  saveMode,
  setAppearance,
  setLaunchAtLogin,
  setSettings,
  type AppleFmAvailability,
  type CaseStyle,
  type DevBinaryInfo,
  type DiagnoseHotkeys,
  type HotkeyDiagnostic,
  type InferenceServerStatus,
  type ModelStatus,
  type ParakeetAvailability,
  type PunctuationStyle,
  type Settings,
  type UpdateCheckResult,
  type VoiceMode,
} from "../lib/ipc";
import { VoxLogo } from "../components/VoxLogo";

type Tab =
  | "general"
  | "hotkeys"
  | "models"
  | "modes"
  | "dictionary"
  | "privacy"
  | "about";

const TABS: { id: Tab; label: string; icon: string; subtitle: string }[] = [
  {
    id: "general",
    label: "General",
    icon: "sliders",
    subtitle: "How Vox starts, listens, and looks.",
  },
  {
    id: "hotkeys",
    label: "Hotkeys",
    icon: "keyboard",
    subtitle: "Shortcuts for dictating, cancelling, and switching modes.",
  },
  {
    id: "models",
    label: "Models",
    icon: "cube",
    subtitle: "Manage the on-device speech and polishing engines.",
  },
  {
    id: "modes",
    label: "Modes",
    icon: "modes",
    subtitle: "Tone, punctuation, and dictionary per situation.",
  },
  {
    id: "dictionary",
    label: "Dictionary",
    icon: "book",
    subtitle: "Names and acronyms Vox should always spell correctly.",
  },
  {
    id: "privacy",
    label: "Privacy",
    icon: "lock",
    subtitle: "What stays on this Mac (everything).",
  },
  {
    id: "about",
    label: "About",
    icon: "info",
    subtitle: "Version, diagnostics, and updates.",
  },
];

interface HotkeyOption {
  value: string;
  label: string;
  /** When set, the picker renders a ⚠︎ glyph and tooltip. Right-Option
   * falls in this bucket: `tauri-plugin-global-shortcut` reports an
   * "Unknown scancode for AltRight" on most macOS keymaps. */
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

// Common languages that aren't in Parakeet's 25-language set. Mirrors
// `NON_PARAKEET_COMMON` in Onboarding.tsx so both surfaces offer the
// same coverage.
const NON_PARAKEET_COMMON: readonly string[] = [
  "zh", "ja", "ko", "ar", "hi", "tr", "vi", "th",
] as const;

const LANGUAGE_OPTIONS: { value: string; label: string }[] = [
  { value: "auto", label: "Auto (detect)" },
  ...[
    ...PARAKEET_SUPPORTED_LANGUAGES,
    ...NON_PARAKEET_COMMON,
  ]
    .map((code) => ({ value: code, label: `${languageDisplayName(code)} (${code})` }))
    .sort((a, b) => a.label.localeCompare(b.label)),
];

export default function SettingsView() {
  const [tab, setTab] = createSignal<Tab>("general");
  const [loaded, setLoaded] = createSignal(false);
  const [state, setState] = createSignal<Settings | null>(null);
  const [savedFlash, setSavedFlash] = createSignal(false);

  let unlistenSettings: (() => void) | null = null;
  // Bumped on local saves so the settings:changed echo can be ignored —
  // a redundant setState would force every controlled input to re-render
  // mid-typing.
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
      setTimeout(() => setSavedFlash(false), 1200);
    } catch (err) {
      console.error("Save failed:", err);
    }
  }

  return (
    <div class="liquid-canvas flex h-screen w-screen text-[var(--color-ink)]">
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
        <SidebarLogo />
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
          title="Audio and transcripts never leave this Mac."
        >
          Local
        </span>
      </div>
    </aside>
  );
}

function SidebarLogo() {
  return <VoxLogo size={28} />;
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
  const meta = createMemo(() => TABS.find((t) => t.id === props.tab));
  return (
    <header class="mb-5 flex items-end justify-between gap-4 border-b border-[var(--color-border)] pb-4">
      <div class="flex flex-col">
        <h1 class="text-[20px] font-semibold tracking-[-0.015em] text-[var(--color-ink)]">
          {meta()?.label ?? ""}
        </h1>
        <p class="mt-0.5 text-[12px] text-[var(--color-ink-mute)]">
          {meta()?.subtitle ?? ""}
        </p>
      </div>
      <div class="flex h-[22px] items-center">
        <span
          class="inline-flex items-center gap-1 rounded-full bg-[var(--color-success)]/15 px-2 py-0.5 text-[11px] font-medium text-[var(--color-success)] transition-opacity duration-200"
          classList={{
            "opacity-100": props.savedFlash,
            "opacity-0 pointer-events-none": !props.savedFlash,
          }}
          aria-live="polite"
        >
          <svg
            width="11"
            height="11"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="3"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <polyline points="5 12 10 17 19 7" />
          </svg>
          Saved
        </span>
      </div>
    </header>
  );
}

function Card(props: { children: any }) {
  return (
    <div class="liquid-card mb-5 overflow-hidden rounded-[14px] ring-1 ring-[var(--color-border)]">
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
      <div class="flex min-w-0 flex-1 flex-col">
        <span class="text-[13px] text-[var(--color-ink)]">{props.label}</span>
        <Show when={props.hint}>
          <span class="text-[11px] text-[var(--color-ink-mute)]">
            {props.hint}
          </span>
        </Show>
      </div>
      <div class="flex shrink-0 items-center gap-2">{props.children}</div>
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
      class="relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full ring-1 ring-inset transition-colors"
      classList={{
        "bg-[var(--color-accent)] ring-transparent": props.checked,
        "bg-[var(--color-control)] ring-[var(--color-border-strong)]":
          !props.checked,
      }}
    >
      <span
        class="toggle-knob inline-block h-[18px] w-[18px] transform rounded-full transition-transform"
        classList={{
          "translate-x-[18px]": props.checked,
          "translate-x-[2px]": !props.checked,
        }}
      />
    </button>
  );
}

function SegmentedControl<T extends string>(props: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  ariaLabel?: string;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={props.ariaLabel}
      class="inline-flex shrink-0 items-center rounded-[8px] bg-[var(--color-control)] p-[2px] ring-1 ring-inset ring-[var(--color-border)]"
    >
      <For each={props.options}>
        {(o) => {
          const active = () => props.value === o.value;
          return (
            <button
              type="button"
              role="radio"
              aria-checked={active()}
              onClick={() => props.onChange(o.value)}
              class="rounded-[6px] px-3 py-[3px] text-[12px] font-medium leading-[16px] transition-all"
              classList={{
                "text-[var(--color-ink)]": active(),
                "text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]":
                  !active(),
              }}
              style={
                active()
                  ? {
                      background: "var(--color-surface-raised)",
                      // Contained shadow: a 1px inset highlight + a 1px
                      // hairline drop. The full glass-shadow-stack-sm
                      // (which has a 22px outer blur) was leaking past
                      // the pill's 2px bottom padding and reading as
                      // asymmetric padding on the active segment.
                      "box-shadow":
                        "inset 0 1px 0 var(--glass-highlight), 0 1px 1px var(--glass-shadow-contact)",
                    }
                  : {}
              }
            >
              {o.label}
            </button>
          );
        }}
      </For>
    </div>
  );
}

function SectionHeader(props: { title: string; hint?: string }) {
  return (
    <div class="mt-1 mb-2 flex items-baseline justify-between gap-3 px-1">
      <h2 class="text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--color-ink-mute)]">
        {props.title}
      </h2>
      <Show when={props.hint}>
        <span class="text-[11px] text-[var(--color-ink-faint)]">
          {props.hint}
        </span>
      </Show>
    </div>
  );
}

function Keycap(props: { children: any }) {
  return <span class="keycap">{props.children}</span>;
}

function StaticValue(props: { children: any }) {
  return (
    <span class="text-[12px] font-medium text-[var(--color-ink-mute)]">
      {props.children}
    </span>
  );
}

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

  async function changeAppearance(v: "light" | "dark") {
    try {
      await setAppearance(v);
      props.update({ appearance: v });
    } catch (err) {
      console.error("Appearance change failed:", err);
    }
  }

  return (
    <>
      <SectionHeader title="Recording" />
      <Card>
        <Row
          label="When you press the hotkey"
          hint="Hands-free toggles on a press; push-to-talk records while you hold."
          last
        >
          <SegmentedControl<"toggle" | "ptt">
            ariaLabel="Hotkey behavior"
            value={props.s.hands_free_enabled ? "toggle" : "ptt"}
            options={[
              { value: "toggle", label: "Hands-free" },
              { value: "ptt", label: "Push-to-talk" },
            ]}
            onChange={(v) =>
              props.update({ hands_free_enabled: v === "toggle" })
            }
          />
        </Row>
      </Card>

      <SectionHeader title="Voice" />
      <Card>
        <Row
          label="Spoken language"
          hint={`${friendlySttEngineName(props.s.stt_engine)} detects the language automatically when set to Auto.`}
        >
          <select
            class="rounded-[8px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border-strong)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s.language}
            onChange={(e) => props.update({ language: e.currentTarget.value })}
          >
            <For each={LANGUAGE_OPTIONS}>
              {(o) => <option value={o.value}>{o.label}</option>}
            </For>
          </select>
        </Row>
        <Row
          label="Suggest mode for the focused app"
          hint="Vox hints, never auto-switches."
        >
          <Toggle
            checked={props.s.auto_suggest_modes}
            onChange={(v) => props.update({ auto_suggest_modes: v })}
          />
        </Row>
        <Row
          label="Microphone"
          hint="Vox follows the macOS default input — change it in System Settings → Sound."
          last
        >
          <StaticValue>System default</StaticValue>
        </Row>
      </Card>

      <SectionHeader title="Appearance" />
      <Card>
        <Row label="Theme" hint="Switches all Vox windows immediately." last>
          <SegmentedControl<"light" | "dark">
            ariaLabel="Theme"
            value={props.s.appearance}
            options={[
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
            onChange={(v) => void changeAppearance(v)}
          />
        </Row>
      </Card>

      <SectionHeader title="Streaming pill" />
      <Card>
        <Row
          label="Show streaming pill"
          hint="See the transcript appear live in the pill after you release the hotkey."
          last
        >
          <Toggle
            checked={props.s.streaming_pill_enabled}
            onChange={(v) => props.update({ streaming_pill_enabled: v })}
          />
        </Row>
      </Card>

      <SectionHeader title="Startup" />
      <Card>
        <Row
          label="Launch at login"
          hint="Vox starts quietly in your menu bar after sign-in."
          last
        >
          <Toggle
            checked={props.s.launch_at_login}
            onChange={(v) => void toggleLaunchAtLogin(v)}
          />
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
    // Backend re-registers hotkeys on every save; refetch so the
    // picker reflects the OS truth (including fallback / "Not bound").
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
      <SectionHeader title="Primary shortcut" />
      <Card>
        <div class="flex items-start justify-between gap-4 px-4 py-3">
          <div class="flex flex-col gap-1.5">
            <div class="flex items-center gap-2">
              <span class="text-[13px] font-medium text-[var(--color-ink)]">
                Push-to-talk
              </span>
              <span class="rounded-full bg-[var(--color-ink-mute)]/15 px-2 py-0.5 text-[10px] uppercase tracking-[0.06em] text-[var(--color-ink-mute)]">
                Built-in
              </span>
            </div>
            <div class="flex items-center gap-1">
              <Keycap>⌘</Keycap>
              <span class="text-[10px] text-[var(--color-ink-faint)]">+</span>
              <Keycap>⌥</Keycap>
              <span class="text-[10px] text-[var(--color-ink-faint)]">+</span>
              <Keycap>.</Keycap>
            </div>
            <span class="text-[11px] text-[var(--color-ink-mute)]">
              {pttHint(props.s.hands_free_enabled, ptt())}
            </span>
          </div>
          <div class="flex shrink-0 flex-col items-end gap-1">
            <button
              type="button"
              onClick={() => void onTest()}
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] hover:bg-[var(--color-control-hover)]"
              title="Read the current binding from the runtime"
            >
              Test binding
            </button>
            <Show when={testStatus()}>
              <span
                class="text-[10px]"
                classList={{
                  "text-[var(--color-success)]":
                    testStatus()?.startsWith("Bound") ?? false,
                  "text-[var(--color-danger)]":
                    testStatus()?.startsWith("Failed") ?? false,
                }}
              >
                {testStatus()}
              </span>
            </Show>
          </div>
        </div>
      </Card>

      <SectionHeader title="Secondary shortcuts" />
      <Card>
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
        Changes apply immediately. If the configured key fails to bind on
        macOS, Vox falls back to the next compatible shortcut and shows
        it under the picker.
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
  // v1 picker options never include modifier-only chords, so the
  // chord-Accessibility branch is unreachable. Keep the helper for the
  // diagnostic snapshot but render a single "Not bound" chip.
  const failedChip = () => "Not bound";
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
      </div>
      <div class="flex shrink-0 items-center gap-2">
        <select
          class="rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
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

function friendlySttEngineName(engine: string): string {
  if (engine === "parakeet-local") return "Parakeet";
  if (engine === "whisper-local") return "Whisper";
  return "The active engine";
}

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
    const dropDownloading = (id: string) =>
      setDownloading((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    const dropProgress = (id: string) =>
      setProgress((prev) => {
        const { [id]: _, ...rest } = prev;
        return rest;
      });

    unlistens.push(
      await onModelDone(async (p) => {
        dropDownloading(p.id);
        dropProgress(p.id);
        await refresh();
      }),
    );
    unlistens.push(
      await onModelError((p) => {
        console.error("model error", p);
        dropDownloading(p.id);
      }),
    );
    unlistens.push(
      await onModelCanceled((p) => {
        dropDownloading(p.id);
        dropProgress(p.id);
      }),
    );
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
  });

  async function startDownload(id: string) {
    setDownloading((prev) => new Set(prev).add(id));
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

  // ModelRow gates the destructive call behind an inline two-step
  // confirm — the native browser `window.confirm()` was silently
  // returning false inside Tauri's WKWebView, so the previous
  // `if (!confirm(...)) return` short-circuited every time and the
  // delete never fired.
  async function remove(id: string) {
    try {
      await deleteModel(id);
    } catch (e) {
      console.error("delete:", e);
    }
    await refresh();
  }

  const [appleFm, setAppleFm] = createSignal<AppleFmAvailability | null>(null);
  const [parakeet, setParakeet] = createSignal<ParakeetAvailability | null>(
    null,
  );

  async function refreshAppleFm() {
    try {
      setAppleFm(await appleFmAvailability());
    } catch (e) {
      console.warn("appleFmAvailability:", e);
    }
  }

  async function refreshParakeet() {
    try {
      setParakeet(await parakeetAvailability());
    } catch (e) {
      console.warn("parakeetAvailability:", e);
    }
  }

  onMount(() => {
    refreshAppleFm();
    refreshParakeet();
  });

  // Switching to a model from a different STT engine flips both fields
  // in one update so session.rs always sees a consistent (engine, model)
  // pair — otherwise a transient mismatch would route the next session
  // to the wrong engine.
  function setActiveStt(engine: string, modelId: string) {
    props.update({ stt_engine: engine, stt_model_id: modelId });
  }

  // One-shot upgrade path: download Parakeet (if not already on disk)
  // and flip the engine. The Use button on the Parakeet model row
  // handles the post-download switch; this just kicks the download.
  async function tryParakeet() {
    const target = sttModels().find(
      (m) => m.descriptor.id === "parakeet-tdt-0.6b-v3",
    );
    if (!target) return;
    if (target.installed) {
      setActiveStt("parakeet-local", target.descriptor.id);
      return;
    }
    await startDownload(target.descriptor.id);
  }

  function setActiveLlm(modelId: string) {
    // Switching to a Gemma model implicitly switches the engine too —
    // a user clicking "Use" on a Gemma row probably wants it active,
    // not "selected for some future toggle".
    props.update({ llm_engine: "gemma-local", llm_model_id: modelId });
  }

  function setEngineAppleFm() {
    // Re-probe right before switching so we surface the current
    // availability error if the user toggled Apple Intelligence off in
    // System Settings since the tab was last opened.
    refreshAppleFm();
    props.update({ llm_engine: "apple-fm" });
  }

  function setEngineGemma() {
    // Use the existing llm_model_id if it still maps to a Gemma model,
    // otherwise the field stays as-is and the user picks one below.
    props.update({ llm_engine: "gemma-local" });
  }

  const sttModels = createMemo(() =>
    models().filter((m) => m.descriptor.kind === "stt"),
  );
  const llmModels = createMemo(() =>
    models().filter((m) => m.descriptor.kind === "llm"),
  );

  // `polishing_enabled` carries user *intent* across LLM outages so we
  // can restore the engine when it stabilises. `llm_engine` carries the
  // engine choice (apple-fm vs gemma-local) and survives the toggle —
  // session.rs short-circuits to the no-op when polishing_enabled is
  // false, so we don't need to clobber the field on toggle off.
  function setPolishing(on: boolean) {
    const next: Partial<Settings> = { polishing_enabled: on };
    if (on && props.s.llm_engine === "none") {
      // Cloud-migration / fresh-install rescue: pick a sensible engine
      // when turning polishing back on with no engine remembered.
      next.llm_engine = appleFm()?.available ? "apple-fm" : "gemma-local";
    }
    props.update(next);
  }

  const activeStt = createMemo(() =>
    sttModels().find((m) => m.descriptor.id === props.s.stt_model_id),
  );
  const activeLlm = createMemo(() =>
    llmModels().find((m) => m.descriptor.id === props.s.llm_model_id),
  );

  // Partition the STT catalog by engine so the two engine cards can
  // each render only their own model rows.
  const parakeetModels = createMemo(() =>
    sttModels().filter((m) => m.descriptor.engine === "parakeet-local"),
  );
  const whisperModels = createMemo(() =>
    sttModels().filter((m) => m.descriptor.engine === "whisper-local"),
  );

  // The "Try Parakeet" upgrade prompt only fires when (a) the user
  // hasn't dismissed it, (b) Parakeet is actually available on this
  // Mac, (c) the user is currently on Whisper, and (d) their active
  // language sits inside Parakeet's 25-language coverage. "auto"
  // qualifies — Parakeet handles auto-detect across the supported set.
  const showTryParakeet = createMemo(() => {
    if (props.s.try_parakeet_dismissed) return false;
    if (props.s.stt_engine !== "whisper-local") return false;
    if (!parakeet()?.available) return false;
    const lang = props.s.language;
    if (lang === "auto") return true;
    return parakeetSupportsLanguage(lang);
  });

  function dismissTryParakeet() {
    props.update({ try_parakeet_dismissed: true });
  }

  return (
    <>
      <SectionHeader
        title="Speech recognition"
        hint={
          props.s.stt_engine === "parakeet-local"
            ? "Local · Parakeet"
            : "Local · Whisper"
        }
      />
      <Card>
        <Row
          label="Active model"
          hint={
            activeStt()
              ? `${activeStt()!.descriptor.name} · ${activeStt()!.installed ? "installed" : "not yet downloaded"}`
              : "No model selected. Pick one below."
          }
          last
        >
          <StaticValue>
            {activeStt() ? activeStt()!.descriptor.tier : "—"} tier
          </StaticValue>
        </Row>
      </Card>

      <Show when={showTryParakeet()}>
        <div class="mb-3 flex items-start justify-between gap-3 rounded-[10px] bg-[var(--color-accent-soft)] p-3 ring-1 ring-[var(--color-accent)]/40">
          <div class="flex-1 text-[12px] text-[var(--color-ink)]">
            <span class="font-medium">Try Parakeet</span> — typically
            5-10× faster on your language. Runs on the Apple Neural
            Engine.
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:brightness-110"
              onClick={tryParakeet}
            >
              Switch to Parakeet
            </button>
            <button
              type="button"
              aria-label="Dismiss"
              title="Dismiss"
              class="grid h-6 w-6 place-items-center rounded-[6px] text-[14px] leading-none text-[var(--color-ink-mute)] hover:bg-[var(--color-control-hover)] hover:text-[var(--color-ink)]"
              onClick={dismissTryParakeet}
            >
              ✕
            </button>
          </div>
        </div>
      </Show>

      <Show when={parakeet()}>
        {(pa) => (
          <SttEngineCard
            title="Parakeet (NVIDIA)"
            subtitle="NVIDIA Parakeet TDT 0.6B v3 — multilingual, 25 languages, ANE-accelerated."
            recommendation="Recommended for English and European languages."
            engineId="parakeet-local"
            isActive={props.s.stt_engine === "parakeet-local"}
            available={pa().available}
            unavailableReason={pa().reason}
            note="Custom dictionary biasing isn't supported on Parakeet in this version. Switch to Whisper if you rely on the Dictionary tab for technical vocabulary."
          >
            <For each={parakeetModels()}>
              {(m) => (
                <ModelRow
                  m={m}
                  isActive={
                    props.s.stt_engine === "parakeet-local"
                    && props.s.stt_model_id === m.descriptor.id
                  }
                  progress={progress()[m.descriptor.id]}
                  downloading={downloading().has(m.descriptor.id)}
                  onUse={() =>
                    setActiveStt("parakeet-local", m.descriptor.id)
                  }
                  onDownload={() => startDownload(m.descriptor.id)}
                  onCancel={() => cancel(m.descriptor.id)}
                  onDelete={() => remove(m.descriptor.id)}
                />
              )}
            </For>
          </SttEngineCard>
        )}
      </Show>

      <SttEngineCard
        title="Whisper (OpenAI)"
        subtitle="OpenAI Whisper — universal multilingual coverage, broader language support."
        recommendation="Recommended for languages outside Parakeet's 25-language set."
        engineId="whisper-local"
        isActive={props.s.stt_engine === "whisper-local"}
        available={true}
        unavailableReason={null}
        note={null}
      >
        <For each={whisperModels()}>
          {(m) => (
            <ModelRow
              m={m}
              isActive={
                props.s.stt_engine === "whisper-local"
                && props.s.stt_model_id === m.descriptor.id
              }
              progress={progress()[m.descriptor.id]}
              downloading={downloading().has(m.descriptor.id)}
              onUse={() => setActiveStt("whisper-local", m.descriptor.id)}
              onDownload={() => startDownload(m.descriptor.id)}
              onCancel={() => cancel(m.descriptor.id)}
              onDelete={() => remove(m.descriptor.id)}
            />
          )}
        </For>
      </SttEngineCard>

      <SectionHeader
        title="Transcript polishing"
        hint={props.s.polishing_enabled ? "Local · Gemma" : "Off"}
      />
      <Card>
        <Row
          label="Clean up each transcript"
          hint="Fix punctuation, drop filler words, and apply the active Voice Mode."
          last={
            !(props.s.polishing_enabled && props.s.llm_engine === "gemma-local")
          }
        >
          <Toggle
            checked={props.s.polishing_enabled}
            onChange={(v) => setPolishing(v)}
          />
        </Row>
        <Show
          when={
            props.s.polishing_enabled && props.s.llm_engine === "gemma-local"
          }
        >
          <Row
            label="Thinking"
            hint="Let Gemma reason silently before polishing. Slower, but can improve grammar on tricky inputs. Off by default."
            last
          >
            <Toggle
              checked={props.s.enable_thinking}
              onChange={(v) => props.update({ enable_thinking: v })}
            />
          </Row>
        </Show>
      </Card>
      <Show when={props.s.polishing_enabled}>
        <SectionHeader
          title="Polishing engine"
          hint={
            props.s.llm_engine === "apple-fm"
              ? "Apple Intelligence"
              : "Gemma 4 (local)"
          }
        />
        <Show when={appleFm()?.supported_host}>
          <AppleFmEngineRow
            availability={appleFm()}
            isActive={props.s.llm_engine === "apple-fm"}
            onUse={setEngineAppleFm}
            onRefresh={refreshAppleFm}
          />
        </Show>
        <GemmaEngineRow
          isActive={props.s.llm_engine === "gemma-local"}
          onUse={setEngineGemma}
        />
        <Show when={props.s.llm_engine === "gemma-local"}>
          <Show when={!props.s.gemma_tos_accepted}>
            <div class="mb-3 rounded-[10px] bg-[var(--color-accent-soft)] p-3 ring-1 ring-[var(--color-accent)]/40 text-[12px] text-[var(--color-ink)]">
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
          <InferenceServerSection />
          <SectionHeader
            title="Polishing model"
            hint={activeLlm()?.descriptor.tier ? `${activeLlm()!.descriptor.tier} tier` : undefined}
          />
          <For each={llmModels()}>
            {(m) => (
              <ModelRow
                m={m}
                isActive={
                  props.s.llm_engine === "gemma-local" &&
                  props.s.llm_model_id === m.descriptor.id
                }
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
      </Show>
    </>
  );
}

// Wraps a group of STT model rows under a labeled card with the engine
// name, description, and (when applicable) a disabled-reason banner +
// a one-line caveat note. Mirrors the visual hierarchy of the Apple
// FM / Gemma cards on the LLM side so the two sections feel
// symmetrical.
function SttEngineCard(props: {
  title: string;
  subtitle: string;
  recommendation: string;
  engineId: "parakeet-local" | "whisper-local";
  isActive: boolean;
  /** When false, the card is rendered disabled and the body (model
   *  rows) is hidden. */
  available: boolean;
  unavailableReason: string | null;
  /** Optional caveat shown beneath the engine description. */
  note: string | null;
  children: any;
}) {
  return (
    <div
      class="mb-3 rounded-[14px] bg-[var(--color-surface-raised)] p-3 ring-1 ring-[var(--color-border)]"
      classList={{ "opacity-60": !props.available }}
      aria-disabled={!props.available}
    >
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-[13px] font-medium text-[var(--color-ink)]">
              {props.title}
            </span>
            <Show when={props.isActive && props.available}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Active
              </span>
            </Show>
            <Show when={!props.available}>
              <span class="rounded-full bg-[var(--color-ink-faint)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-ink-mute)]">
                Unavailable
              </span>
            </Show>
          </div>
          <p class="mt-1 text-[11px] text-[var(--color-ink-mute)]">
            {props.subtitle}
          </p>
          <p class="mt-0.5 text-[11px] text-[var(--color-ink-faint)]">
            {props.recommendation}
          </p>
          <Show when={!props.available && props.unavailableReason}>
            <p class="mt-1 text-[11px] text-[var(--color-danger)]">
              {props.unavailableReason}
            </p>
          </Show>
          <Show when={props.note && props.available}>
            <p class="mt-2 rounded-[6px] bg-[var(--color-ink-faint)]/10 px-2 py-1 text-[11px] text-[var(--color-ink-mute)]">
              Note: {props.note}
            </p>
          </Show>
        </div>
      </div>
      <Show when={props.available}>
        <div class="mt-3">{props.children}</div>
      </Show>
    </div>
  );
}

function AppleFmEngineRow(props: {
  availability: AppleFmAvailability | null;
  isActive: boolean;
  onUse: () => void;
  onRefresh: () => void;
}) {
  const a = () => props.availability;
  const reachable = () => a()?.available ?? false;
  return (
    <div class="mb-3 rounded-[14px] bg-[var(--color-surface-raised)] p-3 ring-1 ring-[var(--color-border)]">
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1">
          <div class="flex items-center gap-2">
            <span class="text-[13px] font-medium text-[var(--color-ink)]">
              Apple Intelligence
            </span>
            <Show when={props.isActive}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Active
              </span>
            </Show>
            <Show when={!props.isActive && reachable()}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Recommended
              </span>
            </Show>
          </div>
          <p class="mt-1 text-[11px] text-[var(--color-ink-mute)]">
            On-device, no download. Uses Apple's Foundation Models on
            macOS 26+ Apple Silicon Macs.
          </p>
          <Show when={!reachable() && a()}>
            <p class="mt-1 text-[11px] text-[var(--color-danger)]">
              Unavailable: {a()!.message}
            </p>
          </Show>
        </div>
        <div class="flex shrink-0 flex-col items-end gap-1">
          <Show
            when={!props.isActive}
            fallback={
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Active
              </span>
            }
          >
            <button
              type="button"
              disabled={!reachable()}
              onClick={props.onUse}
              class="rounded-[8px] bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
            >
              Use
            </button>
          </Show>
          <Show when={!reachable()}>
            <button
              type="button"
              onClick={props.onRefresh}
              class="rounded-[8px] bg-[var(--color-control)] px-3 py-1 text-[11px] font-medium text-[var(--color-ink)] hover:bg-[var(--color-control-hover)]"
            >
              Re-check
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}

function GemmaEngineRow(props: { isActive: boolean; onUse: () => void }) {
  return (
    <div class="mb-3 rounded-[14px] bg-[var(--color-surface-raised)] p-3 ring-1 ring-[var(--color-border)]">
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1">
          <div class="flex items-center gap-2">
            <span class="text-[13px] font-medium text-[var(--color-ink)]">
              Gemma 4 (open-source)
            </span>
            <Show when={props.isActive}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Active
              </span>
            </Show>
          </div>
          <p class="mt-1 text-[11px] text-[var(--color-ink-mute)]">
            Runs offline once downloaded. Pick a model size below.
          </p>
        </div>
        <div class="flex shrink-0">
          <Show
            when={!props.isActive}
            fallback={
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Active
              </span>
            }
          >
            <button
              type="button"
              onClick={props.onUse}
              class="rounded-[8px] bg-[var(--color-control)] px-3 py-1 text-[11px] font-medium text-[var(--color-ink)] hover:bg-[var(--color-control-hover)]"
            >
              Use
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}

function formatBytes(b: number) {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(0)} KB`;
  if (b < 1024 * 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MB`;
  return `${(b / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

// Sits above the cleanup-models list because the binary is the
// prerequisite — without it, GGUFs on disk do nothing. Polish requests
// fail-open in the orchestrator, so this row exists for the user
// trying to figure out *why* polish stopped working.
function InferenceServerSection() {
  const [status, setStatus] = createSignal<InferenceServerStatus | null>(null);
  const [progress, setProgress] = createSignal<{
    downloaded: number;
    total: number;
    eta: number | null;
  } | null>(null);
  const [downloading, setDownloading] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);

  let unlistens: (() => void)[] = [];

  async function refresh() {
    try {
      setStatus(await inferenceServerStatus());
    } catch (e) {
      console.error("inferenceServerStatus:", e);
    }
  }

  onMount(async () => {
    await refresh();
    unlistens.push(
      await onBinaryProgress((p) => {
        setProgress({
          downloaded: p.bytes_downloaded,
          total: p.total_bytes,
          eta: p.eta_secs,
        });
      }),
    );
    unlistens.push(
      await onBinaryDone(async () => {
        setDownloading(false);
        setProgress(null);
        setErr(null);
        await refresh();
      }),
    );
    unlistens.push(
      await onBinaryError((p) => {
        setErr(p.error);
        setDownloading(false);
      }),
    );
    unlistens.push(
      await onBinaryCanceled(() => {
        setDownloading(false);
        setProgress(null);
      }),
    );
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
  });

  async function install() {
    setDownloading(true);
    setErr(null);
    try {
      await downloadInferenceServer();
    } catch (e) {
      setErr(String(e));
      setDownloading(false);
    }
  }

  // Inline two-step confirm. `window.confirm` is a silent no-op in
  // Tauri's WKWebView so we render a Confirm/Cancel pair on the row
  // instead.
  const [reinstallConfirming, setReinstallConfirming] = createSignal(false);
  let reinstallTimer: ReturnType<typeof setTimeout> | undefined;
  function armReinstallConfirm() {
    setReinstallConfirming(true);
    if (reinstallTimer) clearTimeout(reinstallTimer);
    reinstallTimer = setTimeout(() => setReinstallConfirming(false), 5000);
  }
  function cancelReinstallConfirm() {
    setReinstallConfirming(false);
    if (reinstallTimer) clearTimeout(reinstallTimer);
  }
  async function reinstall() {
    cancelReinstallConfirm();
    try {
      await deleteInferenceServer();
      await refresh();
      await install();
    } catch (e) {
      console.error("reinstall:", e);
      setErr(String(e));
    }
  }
  onCleanup(() => {
    if (reinstallTimer) clearTimeout(reinstallTimer);
  });

  async function restart() {
    try {
      await restartInferenceServer();
    } catch (e) {
      console.error("restart inference:", e);
    }
  }

  const supported = () => status()?.supported_host ?? false;
  const installed = () => status()?.installed ?? false;
  const version = () => status()?.version ?? "";

  return (
    <Show when={supported()}>
      <SectionHeader
        title="Inference engine"
        hint={installed() ? "Installed" : "Required"}
      />
      <Card>
        <Row
          label={
            version()
              ? `llama-server (llama.cpp build ${version()})`
              : "llama-server"
          }
          hint={
            installed()
              ? "Pre-built, sandboxed inference subprocess. Crashes here can't take down Vox."
              : "Pre-built inference subprocess. Required for transcript polishing."
          }
          last
        >
          <div class="flex items-center gap-2">
            <Show when={!installed() && !downloading()}>
              <button
                type="button"
                class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:brightness-110"
                onClick={install}
              >
                Install
              </button>
            </Show>
            <Show when={installed() && !downloading() && !reinstallConfirming()}>
              <button
                type="button"
                class="rounded-[6px] px-3 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
                onClick={restart}
                title="Stop the running server. The next polish will spawn a fresh one."
              >
                Restart
              </button>
              <button
                type="button"
                class="rounded-[6px] px-3 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
                onClick={armReinstallConfirm}
              >
                Reinstall
              </button>
            </Show>
            <Show when={installed() && !downloading() && reinstallConfirming()}>
              <span class="text-[11px] text-[var(--color-ink-mute)]">
                Reinstall? Re-downloads the binary.
              </span>
              <button
                type="button"
                class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] hover:bg-[var(--color-control-hover)]"
                onClick={cancelReinstallConfirm}
              >
                Cancel
              </button>
              <button
                type="button"
                class="rounded-[6px] bg-[var(--color-danger)] px-3 py-1 text-[11px] font-medium text-white hover:brightness-110"
                onClick={() => void reinstall()}
              >
                Reinstall
              </button>
            </Show>
            <Show when={downloading()}>
              <button
                type="button"
                class="rounded-[6px] px-3 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
                onClick={() => cancelInferenceServerDownload()}
              >
                Cancel
              </button>
            </Show>
          </div>
        </Row>
        <Show when={progress()}>
          {(p) => (
            <div class="px-3 pb-3">
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
        <Show when={err()}>
          <p class="px-3 pb-3 text-[11px] text-[var(--color-danger)]">
            {err()}
          </p>
        </Show>
      </Card>
    </Show>
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
  const blurb = () => modelBlurb(props.m.descriptor.id);

  // Inline two-step confirm. Replaces a native `window.confirm()` that
  // was silently returning false in Tauri's WKWebView so the delete
  // never fired.
  const [confirming, setConfirming] = createSignal(false);
  let dismissTimer: ReturnType<typeof setTimeout> | undefined;
  function armConfirm() {
    setConfirming(true);
    if (dismissTimer) clearTimeout(dismissTimer);
    dismissTimer = setTimeout(() => setConfirming(false), 5000);
  }
  function cancelConfirm() {
    setConfirming(false);
    if (dismissTimer) clearTimeout(dismissTimer);
  }
  function commitDelete() {
    cancelConfirm();
    props.onDelete();
  }
  onCleanup(() => {
    if (dismissTimer) clearTimeout(dismissTimer);
  });

  return (
    <div class="liquid-card mb-3 rounded-[10px] p-4 ring-1 ring-[var(--color-border)]">
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
          <Show when={props.m.installed && !props.isActive && !confirming()}>
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
          <Show when={props.m.installed && !confirming()}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] text-[var(--color-ink-mute)] hover:bg-[var(--color-danger)]/20 hover:text-[var(--color-danger)]"
              onClick={armConfirm}
              title="Delete cached model"
            >
              Delete
            </button>
          </Show>
          <Show when={props.m.installed && confirming()}>
            <span class="text-[11px] text-[var(--color-ink-mute)]">
              Free ~{formatBytes(props.m.descriptor.size_bytes)}?
            </span>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
              onClick={cancelConfirm}
            >
              Keep
            </button>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-danger)] px-3 py-1.5 text-[12px] font-medium text-white hover:brightness-110"
              onClick={commitDelete}
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
    </div>
  );
}

function ModesTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [modes, setModes] = createSignal<VoiceMode[]>([]);
  const [editing, setEditing] = createSignal<VoiceMode | null>(null);
  const [importErr, setImportErr] = createSignal<string | null>(null);
  const [resetMsg, setResetMsg] = createSignal<string | null>(null);
  const [saveErr, setSaveErr] = createSignal<string | null>(null);
  // Inline two-step confirm states. `confirmDeleteId` is the row that's
  // armed for delete; `confirmReset` arms the reset-to-defaults button.
  // `window.confirm` is a silent no-op in Tauri's WKWebView, so we
  // can't depend on it — replace with these inline flags + 5s
  // auto-revert.
  const [confirmDeleteId, setConfirmDeleteId] = createSignal<string | null>(
    null,
  );
  const [confirmReset, setConfirmReset] = createSignal(false);
  let confirmDeleteTimer: ReturnType<typeof setTimeout> | undefined;
  let confirmResetTimer: ReturnType<typeof setTimeout> | undefined;
  function armDeleteConfirm(id: string) {
    setConfirmDeleteId(id);
    if (confirmDeleteTimer) clearTimeout(confirmDeleteTimer);
    confirmDeleteTimer = setTimeout(() => setConfirmDeleteId(null), 5000);
  }
  function cancelDeleteConfirm() {
    setConfirmDeleteId(null);
    if (confirmDeleteTimer) clearTimeout(confirmDeleteTimer);
  }
  function armResetConfirm() {
    setConfirmReset(true);
    if (confirmResetTimer) clearTimeout(confirmResetTimer);
    confirmResetTimer = setTimeout(() => setConfirmReset(false), 5000);
  }
  function cancelResetConfirm() {
    setConfirmReset(false);
    if (confirmResetTimer) clearTimeout(confirmResetTimer);
  }
  onCleanup(() => {
    if (confirmDeleteTimer) clearTimeout(confirmDeleteTimer);
    if (confirmResetTimer) clearTimeout(confirmResetTimer);
  });
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
    cancelDeleteConfirm();
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
    cancelResetConfirm();
    setResetMsg(null);
    try {
      const n = await resetDefaultModes();
      await refresh();
      if (n === 0) {
        setResetMsg("All defaults are already present.");
        setTimeout(() => setResetMsg(null), 4000);
      } else {
        setResetMsg(`Restored ${n} default mode${n === 1 ? "" : "s"}.`);
        setTimeout(() => setResetMsg(null), 4000);
      }
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
      // Backend dedups if this collides with an existing mode.
      id = mode.name
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/(^-|-$)/g, "");
      if (!id) id = `mode-${Date.now()}`;
    }
    // Strip trigger rows where every field is blank — those are
    // half-filled drafts, not real triggers.
    const cleanedTriggers = mode.auto_triggers.filter((t) => {
      const bundle = (t.bundle_id_glob ?? "").trim();
      const title = (t.window_title_substring ?? "").trim();
      return bundle.length > 0 || title.length > 0;
    });
    const toSave: VoiceMode = { ...mode, id, auto_triggers: cleanedTriggers };
    try {
      await saveMode(toSave);
      await refresh();
      setEditing(null);
    } catch (e) {
      console.error("save mode:", e);
      // Surface the error inline rather than via window.alert (no-op in
      // WKWebView). The editor stays open so the user can adjust + retry.
      setSaveErr(String(e));
    }
  }

  return (
    <div onDragOver={onDragOver} onDrop={onDrop}>
      <Card>
        <Row
          label="Active mode"
          hint="Used for the next utterance unless you cycle or pick a different mode."
          last
        >
          <select
            class="rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
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
            confirmingDelete={confirmDeleteId() === m.id}
            onActivate={() => onActivate(m.id)}
            onEdit={() => setEditing(m)}
            onDuplicate={() => onDuplicate(m.id)}
            onArmDelete={() => armDeleteConfirm(m.id)}
            onCancelDelete={cancelDeleteConfirm}
            onConfirmDelete={() => onDelete(m.id)}
            onExport={() => onExport(m)}
          />
        )}
      </For>

      <div class="mt-2 flex flex-wrap gap-2">
        <button
          type="button"
          onClick={onNew}
          class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1.5 text-[12px] font-medium text-[var(--color-on-accent)] hover:brightness-110"
        >
          + New mode
        </button>
        <button
          type="button"
          onClick={() => fileInput?.click()}
          class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
        >
          Import .voxmode
        </button>
        <Show
          when={!confirmReset()}
          fallback={
            <>
              <span class="self-center text-[11px] text-[var(--color-ink-mute)]">
                Restore built-in modes?
              </span>
              <button
                type="button"
                onClick={cancelResetConfirm}
                class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={onReset}
                class="rounded-[6px] bg-[var(--color-danger)] px-3 py-1.5 text-[12px] font-medium text-white hover:brightness-110"
              >
                Reset
              </button>
            </>
          }
        >
          <button
            type="button"
            onClick={armResetConfirm}
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            Reset to defaults
          </button>
        </Show>
        <input
          ref={fileInput}
          type="file"
          accept=".voxmode,.json,application/json"
          class="hidden"
          onChange={onFilePick}
        />
      </div>
      <Show when={resetMsg()}>
        <p class="mt-2 text-[11px] text-[var(--color-ink-mute)]">{resetMsg()}</p>
      </Show>
      <p class="mt-3 text-[11px] text-[var(--color-ink-mute)]">
        Drag a <code class="text-[var(--color-ink)]">.voxmode</code> file
        anywhere on this tab to import. Built-in modes can't be edited
        directly — duplicate one to make changes.
      </p>

      <Show when={editing()}>
        {(m) => (
          <ModeEditor
            mode={m()}
            llmEngine={props.s.llm_engine}
            saveError={saveErr()}
            onCancel={() => {
              setSaveErr(null);
              setEditing(null);
            }}
            onSave={(mode) => {
              setSaveErr(null);
              return onSaveDraft(mode);
            }}
          />
        )}
      </Show>
    </div>
  );
}

function ModeRow(props: {
  m: VoiceMode;
  isActive: boolean;
  confirmingDelete: boolean;
  onActivate: () => void;
  onEdit: () => void;
  onDuplicate: () => void;
  onArmDelete: () => void;
  onCancelDelete: () => void;
  onConfirmDelete: () => void;
  onExport: () => void;
}) {
  return (
    <div class="liquid-card mb-3 rounded-[10px] px-5 py-4 ring-1 ring-[var(--color-border)]">
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
          <Show when={!props.isActive && !props.confirmingDelete}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)]/80 px-3 py-1.5 text-[12px] font-medium text-[var(--color-on-accent)] hover:bg-[var(--color-accent)]"
              onClick={props.onActivate}
            >
              Use
            </button>
          </Show>
          <Show when={props.m.built_in && !props.confirmingDelete}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
              onClick={props.onDuplicate}
              title="Make a custom copy you can edit"
            >
              Duplicate
            </button>
          </Show>
          <Show when={!props.m.built_in && !props.confirmingDelete}>
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
              onDelete={props.onArmDelete}
            />
          </Show>
          <Show when={props.confirmingDelete}>
            <span class="text-[11px] text-[var(--color-ink-mute)]">
              Delete this mode?
            </span>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
              onClick={props.onCancelDelete}
            >
              Keep
            </button>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-danger)] px-3 py-1.5 text-[12px] font-medium text-white hover:brightness-110"
              onClick={props.onConfirmDelete}
            >
              Delete
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}

function ModeOverflowMenu(props: {
  onExport: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
}) {
  const [open, setOpen] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;

  function onDocClick(e: MouseEvent) {
    if (!containerRef) return;
    if (!containerRef.contains(e.target as Node)) setOpen(false);
  }

  onMount(() => document.addEventListener("mousedown", onDocClick));
  onCleanup(() => document.removeEventListener("mousedown", onDocClick));

  return (
    <div ref={containerRef} class="relative">
      <button
        type="button"
        aria-label="More actions"
        class="flex h-[28px] w-[28px] items-center justify-center rounded-[6px] bg-[var(--color-control)] text-[var(--color-ink-mute)] hover:bg-[var(--color-control-hover)] hover:text-[var(--color-ink)]"
        onClick={() => setOpen(!open())}
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="currentColor"
          aria-hidden="true"
        >
          <circle cx="5" cy="12" r="1.6" />
          <circle cx="12" cy="12" r="1.6" />
          <circle cx="19" cy="12" r="1.6" />
        </svg>
      </button>
      <Show when={open()}>
        <div
          class="absolute right-0 top-[32px] z-30 flex min-w-[160px] flex-col overflow-hidden rounded-[8px] ring-1 ring-[var(--color-border-strong)]"
          style={{
            background: "var(--color-surface-raised)",
            "backdrop-filter": "var(--glass-blur)",
            "-webkit-backdrop-filter": "var(--glass-blur)",
            "box-shadow": "var(--glass-shadow-stack-sm)",
          }}
          role="menu"
        >
          <MenuItem
            onClick={() => {
              setOpen(false);
              props.onDuplicate();
            }}
          >
            Duplicate
          </MenuItem>
          <MenuItem
            onClick={() => {
              setOpen(false);
              props.onExport();
            }}
          >
            Export…
          </MenuItem>
          <MenuItem
            danger
            onClick={() => {
              setOpen(false);
              props.onDelete();
            }}
          >
            Delete
          </MenuItem>
        </div>
      </Show>
    </div>
  );
}

function MenuItem(props: {
  children: any;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={props.onClick}
      class="px-3 py-1.5 text-left text-[12px] hover:bg-[var(--color-overlay)]"
      classList={{
        "text-[var(--color-ink)]": !props.danger,
        "text-[var(--color-danger)]": props.danger,
      }}
    >
      {props.children}
    </button>
  );
}

function ModeEditor(props: {
  mode: VoiceMode;
  llmEngine: string;
  saveError: string | null;
  onCancel: () => void;
  onSave: (m: VoiceMode) => void;
}) {
  const [draft, setDraft] = createSignal<VoiceMode>({ ...props.mode });
  const [dictDraft, setDictDraft] = createSignal("");
  // Toggled true on a save attempt with an empty required field so the
  // inline error renders without flashing on every keystroke.
  const [showValidation, setShowValidation] = createSignal(false);
  const nameInvalid = () => draft().name.trim().length === 0;
  const promptInvalid = () => draft().system_prompt.trim().length === 0;
  const canSave = () => !nameInvalid() && !promptInvalid();

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
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-[var(--color-scrim)] px-6 py-8">
      <div
        class="flex h-full w-full max-w-2xl flex-col overflow-hidden rounded-[14px] ring-1 ring-[var(--color-border-strong)]"
        style={{
          background: "var(--color-surface-raised)",
          "backdrop-filter": "var(--glass-blur)",
          "-webkit-backdrop-filter": "var(--glass-blur)",
          "box-shadow": "var(--glass-shadow-stack)",
        }}
      >
        <header class="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
          <h2 class="text-[14px] font-semibold">
            {props.mode.id ? "Edit mode" : "New mode"}
          </h2>
          <button
            type="button"
            class="rounded p-1 text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]"
            onClick={props.onCancel}
            aria-label="Cancel"
          >
            ×
          </button>
        </header>
        <div class="flex-1 overflow-y-auto scrollbar-thin px-4 py-4">
          <Card>
            <Row label="Name">
              <div class="flex flex-col items-end gap-1">
                <input
                  type="text"
                  class="w-48 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                  classList={{
                    "ring-[var(--color-danger)]":
                      showValidation() && nameInvalid(),
                  }}
                  value={draft().name}
                  onInput={(e) => patch({ name: e.currentTarget.value })}
                />
                <Show when={showValidation() && nameInvalid()}>
                  <span class="text-[11px] text-[var(--color-danger)]">
                    Name can't be empty.
                  </span>
                </Show>
              </div>
            </Row>
            <Row label="Icon (emoji or text)">
              <input
                type="text"
                class="w-24 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-center text-[14px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().icon}
                onInput={(e) => patch({ icon: e.currentTarget.value })}
              />
            </Row>
            <Row label="Description" last>
              <input
                type="text"
                class="w-64 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().description}
                onInput={(e) => patch({ description: e.currentTarget.value })}
              />
            </Row>
          </Card>

          <Card>
            <div class="px-4 py-3">
              <div class="mb-1 text-[12px] font-medium">System prompt</div>
              <textarea
                class="h-48 w-full rounded-[6px] bg-[var(--color-input-bg)] px-2 py-2 font-mono text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                classList={{
                  "ring-[var(--color-danger)]":
                    showValidation() && promptInvalid(),
                }}
                value={draft().system_prompt}
                onInput={(e) => patch({ system_prompt: e.currentTarget.value })}
              />
              <p class="mt-1 text-[11px] text-[var(--color-ink-mute)]">
                {draft().system_prompt.length} chars. Tip: keep it tight
                and end with explicit "Output only the cleaned text."
              </p>
              <Show when={showValidation() && promptInvalid()}>
                <p class="mt-1 text-[11px] text-[var(--color-danger)]">
                  System prompt can't be empty.
                </p>
              </Show>
            </div>
          </Card>

          <Card>
            <Row
              label="Thinking"
              hint={
                "Override the global Thinking setting for this mode. Only applies when polishing with Gemma." +
                (props.llmEngine !== "gemma-local"
                  ? " (No effect — current polishing engine isn't Gemma.)"
                  : "")
              }
              last
            >
              <select
                class="rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)] disabled:cursor-not-allowed disabled:opacity-50"
                disabled={props.llmEngine !== "gemma-local"}
                value={
                  draft().enable_thinking === true
                    ? "on"
                    : draft().enable_thinking === false
                      ? "off"
                      : "inherit"
                }
                onChange={(e) => {
                  const v = e.currentTarget.value;
                  patch({
                    enable_thinking:
                      v === "on" ? true : v === "off" ? false : null,
                  });
                }}
              >
                <option value="inherit">Inherit from settings</option>
                <option value="on">Always on</option>
                <option value="off">Always off</option>
              </select>
            </Row>
          </Card>

          <Card>
            <Row label="Punctuation">
              <select
                class="rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
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
                class="rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
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
                class="w-20 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
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
                  class="flex-1 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
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
                  class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[12px] hover:bg-[var(--color-control-hover)]"
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
                        class="group inline-flex items-center gap-1 rounded-[6px] bg-[var(--color-control)] px-2 py-1 text-[12px] hover:bg-[var(--color-control-hover)]"
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
                  class="rounded-[6px] bg-[var(--color-control)] px-2 py-1 text-[11px] hover:bg-[var(--color-control-hover)]"
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
                      class="flex-1 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[11px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
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
                      class="flex-1 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[11px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
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
                      class="rounded-[6px] bg-[var(--color-control)] px-2 py-1 text-[11px] hover:bg-[var(--color-danger)]/20 hover:text-[var(--color-danger)]"
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
          <Show when={props.saveError}>
            <span class="mr-auto text-[11px] text-[var(--color-danger)]">
              {props.saveError}
            </span>
          </Show>
          <button
            type="button"
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
            onClick={props.onCancel}
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!canSave()}
            class="rounded-[6px] bg-[var(--color-accent)] px-4 py-1.5 text-[12px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
            onClick={() => {
              if (!canSave()) {
                setShowValidation(true);
                return;
              }
              props.onSave(draft());
            }}
          >
            Save
          </button>
        </footer>
      </div>
    </div>
  );
}

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
              class="flex-1 rounded-[6px] bg-[var(--color-input-bg)] px-3 py-2 text-[13px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
              placeholder="Anthropic, kubectl, OAuth…"
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
              class="rounded-[6px] bg-[var(--color-accent)] px-4 py-2 text-[12px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:opacity-50"
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
                Add a word once and Vox will spell <em>Anthropic</em> even if
                you sound like you're saying <em>Anthropik</em>.
              </p>
            }
          >
            <div class="flex flex-wrap gap-2">
              <For each={props.s.dictionary}>
                {(term) => (
                  <button
                    type="button"
                    onClick={() => remove(term)}
                    class="group inline-flex items-center gap-1 rounded-[6px] bg-[var(--color-control)] px-2 py-1 text-[12px] hover:bg-[var(--color-control-hover)]"
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
  const guarantees: { title: string; detail: string }[] = [
    {
      title: "Audio never leaves the Mac",
      detail:
        "Captured into memory, transcribed locally, freed before the pipeline ends.",
    },
    {
      title: "Transcripts stay on the device",
      detail:
        "Whisper and Gemma both run on Apple Silicon. No network calls in the dictation path.",
    },
    {
      title: "Nothing is written to disk",
      detail:
        "Audio is never saved. Transcripts live only in a process-lifetime ring buffer.",
    },
    {
      title: "No telemetry",
      detail: "No analytics, no crash uploads, no remote logs.",
    },
    {
      title: "No keys to leak",
      detail:
        "There are no API keys to store, because there are no third-party services.",
    },
  ];

  return (
    <>
      <div
        class="liquid-card mb-5 overflow-hidden rounded-[14px] ring-1 ring-[var(--color-border)]"
      >
        <div class="flex items-start gap-3 border-b border-[var(--color-border)] px-4 py-4">
          <div
            class="flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px]"
            style={{
              background: "var(--color-success)",
              "box-shadow":
                "inset 0 1px 0 rgba(255,255,255,0.35), 0 1px 2px rgba(0,0,0,0.15)",
            }}
            aria-hidden="true"
          >
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="white"
              stroke-width="2.25"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <rect x="4" y="11" width="16" height="10" rx="2" />
              <path d="M8 11V7a4 4 0 0 1 8 0v4" />
            </svg>
          </div>
          <div class="flex flex-col">
            <h3 class="text-[14px] font-semibold text-[var(--color-ink)]">
              Fully on-device
            </h3>
            <p class="mt-0.5 text-[12px] text-[var(--color-ink-mute)]">
              Vox runs entirely on this Mac. There is no setting that
              would send your audio or transcripts to any external
              service.
            </p>
          </div>
        </div>
        <ul class="flex flex-col">
          <For each={guarantees}>
            {(g, i) => (
              <li
                class="flex items-start gap-3 px-4 py-3"
                classList={{
                  "border-b border-[var(--color-border)]":
                    i() < guarantees.length - 1,
                }}
              >
                <svg
                  class="mt-0.5 shrink-0 text-[var(--color-success)]"
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"
                >
                  <polyline points="5 12 10 17 19 7" />
                </svg>
                <div class="flex flex-col">
                  <span class="text-[13px] text-[var(--color-ink)]">
                    {g.title}
                  </span>
                  <span class="text-[11px] text-[var(--color-ink-mute)]">
                    {g.detail}
                  </span>
                </div>
              </li>
            )}
          </For>
        </ul>
      </div>
    </>
  );
}

function AboutTab() {
  const [version] = createResource(() => getAppVersion());
  const [checking, setChecking] = createSignal(false);
  const [updateResult, setUpdateResult] = createSignal<UpdateCheckResult | null>(
    null,
  );
  const [diag, setDiag] = createSignal<DiagnoseHotkeys | null>(null);
  const [diagErr, setDiagErr] = createSignal<string | null>(null);
  const [devInfo, setDevInfo] = createSignal<DevBinaryInfo | null>(null);
  const [devCopied, setDevCopied] = createSignal(false);

  // Load dev binary info eagerly so the path chip is visible without
  // requiring the user to click first.
  onMount(async () => {
    try {
      setDevInfo(await revealDevBinaryPath());
    } catch (err) {
      console.warn("revealDevBinaryPath failed:", err);
    }
  });

  // Open System Settings only when the user actually needs it: a dev
  // build (sig rotates, Accessibility entry stale) or Accessibility
  // currently denied. Otherwise just copy the path.
  const shouldAutoOpenAxSettings = () => {
    const d = devInfo();
    if (!d) return false;
    return d.is_dev_build || !d.accessibility_granted;
  };

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
      // Pre-copy so the user can paste into Privacy & Security →
      // Accessibility's Cmd+Shift+G "Go to folder" prompt.
      try {
        await navigator.clipboard.writeText(info.binary_path);
        setDevCopied(true);
      } catch {
        setDevCopied(false);
      }
      if (info.is_dev_build || !info.accessibility_granted) {
        await openAccessibilitySettings();
      }
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
      <div class="liquid-card mb-5 rounded-[14px] p-5 ring-1 ring-[var(--color-border)]">
        <div class="flex items-center gap-3">
          <VoxLogo size={48} />

          <div class="flex flex-col">
            <div class="text-[20px] font-semibold tracking-[-0.015em] text-[var(--color-ink)]">
              Vox
            </div>
            <div class="text-[12px] text-[var(--color-ink-mute)]">
              On-device dictation. Press, speak, paste.
            </div>
          </div>
        </div>
        <div class="mt-3 flex flex-wrap items-center gap-1.5">
          <span class="rounded-full bg-[var(--color-control)] px-2 py-0.5 text-[10px] uppercase tracking-[0.08em] text-[var(--color-ink-mute)]">
            v{version() ?? "—"}
          </span>
          <span class="rounded-full bg-[var(--color-success)]/15 px-2 py-0.5 text-[10px] uppercase tracking-[0.08em] text-[var(--color-success)]">
            Local-first
          </span>
          <span class="rounded-full bg-[var(--color-control)] px-2 py-0.5 text-[10px] uppercase tracking-[0.08em] text-[var(--color-ink-mute)]">
            Tauri 2
          </span>
        </div>
      </div>

      <SectionHeader title="Updates" />
      <Card>
        <Row
          label="Check for updates"
          hint="Auto-update is not yet configured for this build."
          last={!updateResult()}
        >
          <button
            type="button"
            onClick={onCheckUpdates}
            disabled={checking()}
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)] disabled:opacity-40"
          >
            {checking() ? "Checking…" : "Check now"}
          </button>
        </Row>
        <Show when={updateResult()}>
          {(r) => (
            <div class="px-4 py-3">
              <span
                class="inline-flex items-center gap-1.5 text-[12px]"
                classList={{
                  "text-[var(--color-success)]": r().status === "available",
                  "text-[var(--color-ink-mute)]":
                    r().status === "up-to-date" ||
                    r().status === "unconfigured",
                  "text-[var(--color-danger)]": r().status === "error",
                }}
              >
                <Show when={r().status === "unconfigured"}>
                  <svg
                    width="11"
                    height="11"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.25"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                  >
                    <circle cx="12" cy="12" r="9" />
                    <line x1="12" y1="16" x2="12" y2="12" />
                    <line x1="12" y1="8" x2="12.01" y2="8" />
                  </svg>
                </Show>
                {r().message ?? r().status}
              </span>
            </div>
          )}
        </Show>
      </Card>

      <SectionHeader title="Help & diagnostics" />
      <Card>
        <Row
          label="Report a bug"
          hint="Found something broken? Open an issue on GitHub."
        >
          <a
            href="https://github.com/KlymSerhii/vox-releases/issues"
            target="_blank"
            rel="noopener"
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            Open issues
          </a>
        </Row>
        <Row
          label="Replay onboarding"
          hint="Walk through the setup wizard again."
        >
          <button
            type="button"
            onClick={onReplay}
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            Start
          </button>
        </Row>
        <Row
          label="Hotkey diagnostics"
          hint="Print bindings and any registration errors."
        >
          <button
            type="button"
            onClick={() => void onDiagnostics()}
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            Run
          </button>
        </Row>
        <Row
          label="Binary path for Accessibility"
          hint="Dev-build only. macOS keys Accessibility grants by signature, which rotates each rebuild; copy this path to re-add Vox under Privacy & Security if needed."
          last={!devInfo() && !diagErr() && !diag()}
        >
          <button
            type="button"
            onClick={() => void onRevealDevPath()}
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            {shouldAutoOpenAxSettings() ? "Open & copy path" : "Copy path"}
          </button>
        </Row>
        <Show when={devInfo()}>
          {(d) => (
            <div class="flex flex-col gap-1 border-t border-[var(--color-border)] px-4 py-3">
              <span class="text-[11px] text-[var(--color-ink-mute)]">
                Bundle id:{" "}
                <code class="text-[var(--color-ink)]">
                  {d().bundle_identifier}
                </code>
                <Show when={d().is_dev_build}>
                  <span class="ml-2 rounded-full bg-[var(--color-accent)]/20 px-2 py-0.5 text-[10px] text-[var(--color-accent)]">
                    dev build
                  </span>
                </Show>
                <Show when={!d().accessibility_granted}>
                  <span class="ml-2 rounded-full bg-[var(--color-danger)]/20 px-2 py-0.5 text-[10px] text-[var(--color-danger)]">
                    accessibility denied
                  </span>
                </Show>
              </span>
              <span class="break-all text-[11px] text-[var(--color-ink-faint)]">
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
          <div class="border-t border-[var(--color-border)] px-4 py-3 text-[12px] text-[var(--color-danger)]">
            {diagErr()}
          </div>
        </Show>
        <Show when={diag()}>
          {(d) => (
            <div class="border-t border-[var(--color-border)] px-4 py-3">
              <pre class="overflow-x-auto rounded-[6px] bg-[var(--color-input-bg)] p-3 text-[11px] text-[var(--color-ink)]">
                {formatDiagnostics(d())}
              </pre>
            </div>
          )}
        </Show>
      </Card>

      <SectionHeader title="Open-source attributions" />
      <Card>
        <div class="flex flex-col gap-3 px-4 py-3 text-[12px] leading-[1.55] text-[var(--color-ink-mute)]">
          <p>
            Speech recognition powered by{" "}
            <a
              href="https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3"
              target="_blank"
              rel="noopener"
              class="text-[var(--color-accent)] hover:underline"
            >
              NVIDIA Parakeet-TDT-0.6B-v3
            </a>{" "}
            (CC-BY-4.0). CoreML conversion by{" "}
            <a
              href="https://github.com/FluidInference/FluidAudio"
              target="_blank"
              rel="noopener"
              class="text-[var(--color-accent)] hover:underline"
            >
              FluidInference
            </a>{" "}
            (Apache-2.0).
          </p>
          <p>
            Whisper by{" "}
            <a
              href="https://github.com/openai/whisper"
              target="_blank"
              rel="noopener"
              class="text-[var(--color-accent)] hover:underline"
            >
              OpenAI
            </a>{" "}
            (MIT). Inference via{" "}
            <a
              href="https://github.com/ggerganov/whisper.cpp"
              target="_blank"
              rel="noopener"
              class="text-[var(--color-accent)] hover:underline"
            >
              whisper.cpp
            </a>{" "}
            (MIT).
          </p>
          <p>
            Polishing engines: Apple Foundation Models (system) and{" "}
            <a
              href="https://ai.google.dev/gemma"
              target="_blank"
              rel="noopener"
              class="text-[var(--color-accent)] hover:underline"
            >
              Gemma
            </a>{" "}
            by Google (Gemma terms).
          </p>
        </div>
      </Card>
    </>
  );
}

// One-line, user-facing description per model. DESIGN.md §5.6 — explain
// what *changes* for the user (speed / quality trade-offs), not RAM
// numbers (those already live in the metadata row underneath).
function modelBlurb(id: string): string | null {
  switch (id) {
    case "parakeet-tdt-0.6b-v3":
      return "Multilingual (25 languages). Streams transcripts straight onto the Apple Neural Engine.";
    case "whisper-tiny-en-q5_1":
      return "Fastest. English-only. Good for short notes; misses uncommon words.";
    case "whisper-small-en-q5_1":
      return "Fast. English-only. Better with technical terms and names.";
    case "whisper-large-v3-turbo-q5_0":
      return "Multilingual. Sharp accuracy with near-realtime turnaround.";
    case "whisper-large-v3-q5_0":
      return "Multilingual. Best accuracy on long or noisy dictations.";
    case "gemma-4-e2b-q5_k_m":
      return "Snappy cleanup. Punctuation and capitalisation; light filler-word removal.";
    case "gemma-4-e4b-q5_k_m":
      return "Balanced. Reliable filler-word removal and natural sentence flow.";
    case "gemma-4-26b-a4b-q4_k_m":
      return "Slowest. Best at preserving your voice and intent over long dictations.";
    default:
      return null;
  }
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
