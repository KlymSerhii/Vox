import { createSignal, onMount, Show } from "solid-js";
import {
  getSettings,
  setSettings,
  testGroqConnection,
  type Settings,
} from "./lib/ipc";

export default function App() {
  const [groqKey, setGroqKey] = createSignal("");
  const [anthropicKey, setAnthropicKey] = createSignal("");
  const [hotkey, setHotkey] = createSignal("AltRight");
  const [loaded, setLoaded] = createSignal(false);
  const [savedMsg, setSavedMsg] = createSignal<string | null>(null);
  const [testMsg, setTestMsg] = createSignal<{
    ok: boolean;
    text: string;
  } | null>(null);
  const [testing, setTesting] = createSignal(false);
  const [showKey, setShowKey] = createSignal(false);

  onMount(async () => {
    try {
      const s = await getSettings();
      setGroqKey(s.groq_api_key ?? "");
      setAnthropicKey(s.anthropic_api_key ?? "");
      setHotkey(s.hotkey || "AltRight");
    } catch (err) {
      console.error("Failed to load settings:", err);
    } finally {
      setLoaded(true);
    }
  });

  async function save() {
    const next: Settings = {
      groq_api_key: groqKey().trim(),
      anthropic_api_key: anthropicKey().trim(),
      hotkey: hotkey(),
    };
    try {
      await setSettings(next);
      setSavedMsg("Saved.");
      setTimeout(() => setSavedMsg(null), 1500);
    } catch (err) {
      setSavedMsg(`Save failed: ${String(err)}`);
    }
  }

  async function runTest() {
    setTesting(true);
    setTestMsg(null);
    try {
      const result = await testGroqConnection(groqKey().trim());
      setTestMsg({ ok: result.ok, text: result.message });
    } catch (err) {
      setTestMsg({ ok: false, text: String(err) });
    } finally {
      setTesting(false);
    }
  }

  return (
    <div class="flex min-h-screen flex-col bg-[var(--color-bg)] text-[var(--color-ink)]">
      <header class="flex items-center justify-between border-b border-white/8 px-6 py-4">
        <div class="flex flex-col">
          <h1 class="text-[17px] font-semibold">Vox</h1>
          <span class="text-[11px] text-[var(--color-ink-mute)]">
            Talk to any app.
          </span>
        </div>
        <span class="text-[11px] text-[var(--color-ink-mute)]">v0.1.0</span>
      </header>

      <main class="flex-1 px-6 py-6">
        <Show
          when={loaded()}
          fallback={
            <div class="text-[var(--color-ink-mute)]">Loading…</div>
          }
        >
          <section class="mb-6 rounded-[10px] bg-white/4 p-5 ring-1 ring-white/8">
            <h2 class="mb-1 text-[13px] font-semibold uppercase tracking-wider text-[var(--color-ink-mute)]">
              Groq API key
            </h2>
            <p class="mb-3 text-[11px] text-[var(--color-ink-mute)]">
              Used for Whisper transcription. Get a key at
              {" "}
              <span class="text-[var(--color-accent)]">console.groq.com</span>.
            </p>
            <div class="flex gap-2">
              <input
                type={showKey() ? "text" : "password"}
                class="flex-1 rounded-[6px] bg-black/30 px-3 py-2 text-[13px] text-[var(--color-ink)] ring-1 ring-white/8 outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                placeholder="gsk_..."
                value={groqKey()}
                onInput={(e) => setGroqKey(e.currentTarget.value)}
                autocomplete="off"
                spellcheck={false}
              />
              <button
                class="rounded-[6px] bg-white/6 px-3 py-2 text-[12px] text-[var(--color-ink-mute)] hover:bg-white/10"
                onClick={() => setShowKey(!showKey())}
                type="button"
              >
                {showKey() ? "Hide" : "Show"}
              </button>
            </div>
            <div class="mt-3 flex items-center gap-2">
              <button
                class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1.5 text-[12px] font-medium text-black hover:brightness-110 disabled:opacity-50"
                disabled={testing() || !groqKey().trim()}
                onClick={runTest}
                type="button"
              >
                {testing() ? "Testing…" : "Test connection"}
              </button>
              <Show when={testMsg()}>
                {(m) => (
                  <span
                    class={`text-[12px] ${
                      m().ok
                        ? "text-[var(--color-success)]"
                        : "text-[var(--color-danger)]"
                    }`}
                  >
                    {m().text}
                  </span>
                )}
              </Show>
            </div>
          </section>

          <section class="mb-6 rounded-[10px] bg-white/4 p-5 ring-1 ring-white/8">
            <h2 class="mb-1 text-[13px] font-semibold uppercase tracking-wider text-[var(--color-ink-mute)]">
              Anthropic API key
            </h2>
            <p class="mb-3 text-[11px] text-[var(--color-ink-mute)]">
              Optional. Used for Phase 2 LLM cleanup. Safe to leave empty.
            </p>
            <input
              type="password"
              class="w-full rounded-[6px] bg-black/30 px-3 py-2 text-[13px] text-[var(--color-ink)] ring-1 ring-white/8 outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
              placeholder="sk-ant-..."
              value={anthropicKey()}
              onInput={(e) => setAnthropicKey(e.currentTarget.value)}
              autocomplete="off"
              spellcheck={false}
            />
          </section>

          <section class="mb-6 rounded-[10px] bg-white/4 p-5 ring-1 ring-white/8">
            <h2 class="mb-1 text-[13px] font-semibold uppercase tracking-wider text-[var(--color-ink-mute)]">
              Push-to-talk hotkey
            </h2>
            <p class="mb-3 text-[11px] text-[var(--color-ink-mute)]">
              Hold this key to record, release to transcribe. Restart after
              changes.
            </p>
            <select
              class="w-full rounded-[6px] bg-black/30 px-3 py-2 text-[13px] text-[var(--color-ink)] ring-1 ring-white/8 outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
              value={hotkey()}
              onChange={(e) => setHotkey(e.currentTarget.value)}
            >
              <option value="AltRight">Right Option</option>
              <option value="CommandOrControl+Shift+Space">
                Cmd+Shift+Space
              </option>
              <option value="F13">F13</option>
            </select>
          </section>

          <div class="flex items-center gap-3">
            <button
              class="rounded-[6px] bg-[var(--color-accent)] px-4 py-2 text-[13px] font-medium text-black hover:brightness-110"
              onClick={save}
              type="button"
            >
              Save
            </button>
            <Show when={savedMsg()}>
              {(m) => (
                <span class="text-[12px] text-[var(--color-success)]">
                  {m()}
                </span>
              )}
            </Show>
          </div>

          <footer class="mt-8 border-t border-white/8 pt-4 text-[11px] text-[var(--color-ink-mute)]">
            <p>
              macOS requires{" "}
              <strong class="text-[var(--color-ink)]">Microphone</strong> and{" "}
              <strong class="text-[var(--color-ink)]">Accessibility</strong>{" "}
              permissions for dictation. Grant them from System Settings →
              Privacy & Security.
            </p>
          </footer>
        </Show>
      </main>
    </div>
  );
}
