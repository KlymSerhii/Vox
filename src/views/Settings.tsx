import { createMemo, createSignal, For, onMount, Show } from "solid-js";
import {
  getSettings,
  setSettings,
  testGroqConnection,
  testAnthropicConnection,
  replayOnboarding,
  type Settings,
  type TestConnectionResult,
} from "../lib/ipc";

type Tab =
  | "general"
  | "hotkeys"
  | "keys"
  | "dictionary"
  | "privacy"
  | "about";

const TABS: { id: Tab; label: string; icon: string }[] = [
  { id: "general", label: "General", icon: "sliders" },
  { id: "hotkeys", label: "Hotkeys", icon: "keyboard" },
  { id: "keys", label: "API Keys", icon: "key" },
  { id: "dictionary", label: "Dictionary", icon: "book" },
  { id: "privacy", label: "Privacy", icon: "lock" },
  { id: "about", label: "About", icon: "info" },
];

const HOTKEY_OPTIONS: { value: string; label: string }[] = [
  { value: "AltRight", label: "Right Option (⌥)" },
  { value: "CommandOrControl+Shift+Space", label: "⌘⇧Space" },
  { value: "F13", label: "F13" },
  { value: "F14", label: "F14" },
  { value: "F15", label: "F15" },
];

const CANCEL_HOTKEY_OPTIONS: { value: string; label: string }[] = [
  { value: "Escape", label: "Esc" },
  { value: "F13", label: "F13" },
  { value: "F14", label: "F14" },
  { value: "F15", label: "F15" },
];

export default function SettingsView() {
  const [tab, setTab] = createSignal<Tab>("general");
  const [loaded, setLoaded] = createSignal(false);
  const [state, setState] = createSignal<Settings | null>(null);
  const [savedFlash, setSavedFlash] = createSignal(false);

  onMount(async () => {
    try {
      const s = await getSettings();
      setState(s);
    } finally {
      setLoaded(true);
    }
    // Cmd+, should just focus the window — already the case.
  });

  async function update(patch: Partial<Settings>) {
    const cur = state();
    if (!cur) return;
    const next = { ...cur, ...patch } as Settings;
    setState(next);
    try {
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
              <Show when={tab() === "keys"}>
                <KeysTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "dictionary"}>
                <DictionaryTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "privacy"}>
                <PrivacyTab s={s()} update={update} />
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
        Vox 0.1.0
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

// -------- Card/Row primitives --------

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
  return (
    <>
      <Card>
        <Row
          label="Launch at login"
          hint="Vox will start quietly in your menu bar. (Phase 3 — not yet wired.)"
        >
          <Toggle
            checked={props.s.launch_at_login}
            onChange={(v) => props.update({ launch_at_login: v })}
          />
        </Row>
        <Row
          label="Clean up with LLM"
          hint="Polish punctuation, remove filler words via Claude Haiku 4.5."
        >
          <Toggle
            checked={props.s.cleanup_enabled}
            onChange={(v) => props.update({ cleanup_enabled: v })}
          />
        </Row>
        <Row
          label="Play sound on start / stop"
          hint="Subtle audio cues when recording begins and ends. (Stub.)"
          last
        >
          <Toggle
            checked={props.s.play_sound}
            onChange={(v) => props.update({ play_sound: v })}
          />
        </Row>
      </Card>

      <Card>
        <Row
          label="Default microphone"
          hint="Vox uses the current system default input device. (Phase 3 — device picker.)"
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
  return (
    <>
      <Card>
        <Row
          label="Push-to-talk"
          hint="Hold this key to record; release to transcribe."
        >
          <select
            class="rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s.hotkey}
            onChange={(e) =>
              props.update({ hotkey: e.currentTarget.value })
            }
          >
            <For each={HOTKEY_OPTIONS}>
              {(o) => <option value={o.value}>{o.label}</option>}
            </For>
          </select>
        </Row>
        <Row
          label="Cancel while recording"
          hint="Abort the pipeline without pasting."
          last
        >
          <select
            class="rounded-[6px] bg-black/30 px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s.cancel_hotkey}
            onChange={(e) =>
              props.update({ cancel_hotkey: e.currentTarget.value })
            }
          >
            <For each={CANCEL_HOTKEY_OPTIONS}>
              {(o) => <option value={o.value}>{o.label}</option>}
            </For>
          </select>
        </Row>
      </Card>
      <p class="text-[11px] text-[var(--color-ink-mute)]">
        Hotkey changes take effect after the app restarts.
      </p>
    </>
  );
}

function KeysTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  return (
    <>
      <ProviderKey
        title="Groq"
        description="Used for Whisper transcription. Get a key at console.groq.com."
        value={props.s.groq_api_key}
        onChange={(v) => props.update({ groq_api_key: v })}
        tester={testGroqConnection}
      />
      <ProviderKey
        title="Anthropic"
        description="Used for LLM cleanup (Claude Haiku 4.5). Optional."
        value={props.s.anthropic_api_key}
        onChange={(v) => props.update({ anthropic_api_key: v })}
        tester={testAnthropicConnection}
      />
      <p class="text-[11px] text-[var(--color-ink-mute)]">
        Keys are stored in the macOS Keychain under{" "}
        <code class="text-[var(--color-ink)]">app.vox.vox</code>. If Keychain
        access is denied we fall back to an encrypted-at-rest plaintext store.
      </p>
    </>
  );
}

function ProviderKey(props: {
  title: string;
  description: string;
  value: string;
  onChange: (v: string) => void;
  tester: (key: string) => Promise<TestConnectionResult>;
}) {
  const [show, setShow] = createSignal(false);
  const [testing, setTesting] = createSignal(false);
  const [result, setResult] = createSignal<TestConnectionResult | null>(null);

  async function runTest() {
    setTesting(true);
    setResult(null);
    try {
      const r = await props.tester(props.value.trim());
      setResult(r);
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    } finally {
      setTesting(false);
    }
  }

  return (
    <Card>
      <div class="px-4 py-3">
        <div class="mb-2 flex items-center justify-between">
          <span class="text-[13px] font-medium">{props.title}</span>
          <span class="text-[11px] text-[var(--color-ink-mute)]">
            {props.value ? "Stored" : "Not set"}
          </span>
        </div>
        <p class="mb-3 text-[11px] text-[var(--color-ink-mute)]">
          {props.description}
        </p>
        <div class="flex gap-2">
          <input
            type={show() ? "text" : "password"}
            class="flex-1 rounded-[6px] bg-black/30 px-3 py-2 text-[13px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            placeholder={props.title === "Groq" ? "gsk_…" : "sk-ant-…"}
            value={props.value}
            autocomplete="off"
            spellcheck={false}
            onInput={(e) => props.onChange(e.currentTarget.value)}
          />
          <button
            type="button"
            class="rounded-[6px] bg-white/6 px-3 py-2 text-[12px] text-[var(--color-ink-mute)] hover:bg-white/10"
            onClick={() => setShow(!show())}
          >
            {show() ? "Hide" : "Show"}
          </button>
          <button
            type="button"
            class="rounded-[6px] bg-[var(--color-accent)] px-3 py-2 text-[12px] font-medium text-black hover:brightness-110 disabled:opacity-50"
            disabled={testing() || !props.value.trim()}
            onClick={runTest}
          >
            {testing() ? "Testing…" : "Test"}
          </button>
        </div>
        <Show when={result()}>
          {(r) => (
            <div
              class="mt-2 text-[12px]"
              classList={{
                "text-[var(--color-success)]": r().ok,
                "text-[var(--color-danger)]": !r().ok,
              }}
            >
              {r().message}
            </div>
          )}
        </Show>
      </div>
    </Card>
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
            Applied during the LLM cleanup step.
          </p>
          <div class="mb-3 flex gap-2">
            <input
              type="text"
              class="flex-1 rounded-[6px] bg-black/30 px-3 py-2 text-[13px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
              placeholder="e.g. Klymenko, Tauri, gpt-4o"
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
        {props.s.dictionary.length} term{props.s.dictionary.length === 1 ? "" : "s"}
        .
      </p>
    </>
  );
}

function PrivacyTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  return (
    <>
      <Card>
        <Row
          label="Send audio to cloud"
          hint="When off, dictation is disabled — Vox has no local STT yet."
          last
        >
          <Toggle
            checked={props.s.send_audio_to_cloud}
            onChange={(v) => props.update({ send_audio_to_cloud: v })}
          />
        </Row>
      </Card>

      <div class="mb-5 rounded-[10px] bg-[var(--color-surface-raised)] p-4 ring-1 ring-[var(--color-border)]">
        <h3 class="mb-2 text-[13px] font-medium">What leaves your device</h3>
        <ul class="space-y-1 text-[12px] text-[var(--color-ink-mute)]">
          <li>• Audio → Groq (Whisper transcription)</li>
          <li>
            • Transcripts → Anthropic (only if cleanup is enabled and a key is
            set)
          </li>
        </ul>
      </div>

      <div class="mb-5 rounded-[10px] bg-[var(--color-surface-raised)] p-4 ring-1 ring-[var(--color-border)]">
        <h3 class="mb-2 text-[13px] font-medium">What never leaves</h3>
        <ul class="space-y-1 text-[12px] text-[var(--color-ink-mute)]">
          <li>• No audio is ever written to disk.</li>
          <li>• No transcript history is persisted across app quits.</li>
          <li>• No telemetry, no analytics, no crash uploads.</li>
          <li>
            • API keys live in the macOS Keychain under{" "}
            <code class="text-[var(--color-ink)]">app.vox.vox</code>.
          </li>
        </ul>
      </div>
    </>
  );
}

function AboutTab() {
  async function onReplay() {
    try {
      await replayOnboarding();
    } catch (err) {
      console.error(err);
    }
  }
  return (
    <>
      <div class="mb-5 rounded-[10px] bg-[var(--color-surface-raised)] p-5 ring-1 ring-[var(--color-border)]">
        <div class="mb-1 text-[20px] font-semibold">Vox</div>
        <div class="text-[13px] text-[var(--color-ink-mute)]">
          Talk to any app.
        </div>
        <div class="mt-3 text-[11px] text-[var(--color-ink-faint)]">
          Version 0.1.0 · Phase 2 · Built with Tauri
        </div>
      </div>
      <Card>
        <Row label="Replay onboarding" hint="Walk through the setup wizard again.">
          <button
            type="button"
            onClick={onReplay}
            class="rounded-[6px] bg-white/6 px-3 py-1.5 text-[12px] hover:bg-white/10"
          >
            Start
          </button>
        </Row>
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

