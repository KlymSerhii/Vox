import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  checkPermissions,
  finishOnboarding,
  getSettings,
  openAccessibilitySettings,
  requestMicrophone,
  setSettings,
  testAnthropicConnection,
  testGroqConnection,
  type Settings,
  type PermissionStatus,
} from "../lib/ipc";

type Step = 0 | 1 | 2 | 3 | 4;

export default function OnboardingView() {
  const [step, setStep] = createSignal<Step>(0);
  const [s, setS] = createSignal<Settings | null>(null);
  const [perms, setPerms] = createSignal<PermissionStatus>({
    microphone: false,
    accessibility: false,
  });

  let pollTimer: ReturnType<typeof setInterval> | null = null;

  onMount(async () => {
    try {
      const settings = await getSettings();
      setS(settings);
    } catch (e) {
      console.error(e);
    }
    // Permission polling — every 2s while window open.
    const poll = async () => {
      try {
        setPerms(await checkPermissions());
      } catch (e) {
        // Non-fatal.
      }
    };
    poll();
    pollTimer = setInterval(poll, 2000);
  });

  onCleanup(() => {
    if (pollTimer) clearInterval(pollTimer);
  });

  async function patch(p: Partial<Settings>) {
    const cur = s();
    if (!cur) return;
    const next = { ...cur, ...p };
    setS(next);
    await setSettings(next);
  }

  async function handleFinish() {
    await finishOnboarding();
  }

  function next() {
    const n = Math.min(4, step() + 1) as Step;
    setStep(n);
  }

  function back() {
    const n = Math.max(0, step() - 1) as Step;
    setStep(n);
  }

  return (
    <div class="flex h-screen w-screen flex-col bg-[var(--color-bg)] text-[var(--color-ink)]">
      <header class="flex items-center justify-between px-6 pt-5 pb-2">
        <Show
          when={step() > 0}
          fallback={<div class="h-6 w-6" />}
        >
          <button
            type="button"
            onClick={back}
            class="rounded p-1 text-[var(--color-ink-mute)] hover:bg-white/8 hover:text-[var(--color-ink)]"
            aria-label="Back"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="15 18 9 12 15 6" />
            </svg>
          </button>
        </Show>
        <Progress step={step()} />
        <div class="h-6 w-6" />
      </header>

      <main class="flex flex-1 flex-col items-center justify-center px-10">
        <Show when={step() === 0}>
          <Welcome />
        </Show>
        <Show when={step() === 1}>
          <MicStep
            granted={perms().microphone}
            onAllow={async () => {
              await requestMicrophone();
              setPerms(await checkPermissions());
            }}
          />
        </Show>
        <Show when={step() === 2}>
          <AccessibilityStep granted={perms().accessibility} />
        </Show>
        <Show when={step() === 3}>
          <KeysStep s={s()} patch={patch} />
        </Show>
        <Show when={step() === 4}>
          <TryStep />
        </Show>
      </main>

      <footer class="flex items-center justify-between border-t border-[var(--color-border)] px-6 py-4">
        <button
          type="button"
          class="text-[12px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
          onClick={step() === 4 ? handleFinish : next}
        >
          {step() === 0 ? "" : step() === 4 ? "Skip for now" : "Skip"}
        </button>
        <PrimaryButton
          step={step()}
          perms={perms()}
          s={s()}
          onNext={next}
          onFinish={handleFinish}
        />
      </footer>
    </div>
  );
}

function Progress(props: { step: Step }) {
  return (
    <div class="flex items-center gap-2" aria-hidden="true">
      <For each={[0, 1, 2, 3, 4]}>
        {(i) => (
          <span
            class="h-1.5 w-1.5 rounded-full transition-colors"
            classList={{
              "bg-[var(--color-accent)]": props.step === i,
              "bg-white/20": props.step !== i,
            }}
          />
        )}
      </For>
    </div>
  );
}

function PrimaryButton(props: {
  step: Step;
  perms: PermissionStatus;
  s: Settings | null;
  onNext: () => void;
  onFinish: () => void;
}) {
  // PRD §3.9: only the microphone step is a hard gate. Everything else is
  // skippable — the user can always come back to Settings to finish setup.
  const disabled = () => {
    if (props.step === 1) return !props.perms.microphone;
    return false;
  };
  const label = () => {
    if (props.step === 0) return "Get started";
    if (props.step === 4) return "Finish";
    return "Continue";
  };
  return (
    <button
      type="button"
      disabled={disabled()}
      onClick={props.step === 4 ? props.onFinish : props.onNext}
      class="inline-flex h-9 items-center gap-2 rounded-[8px] bg-[var(--color-accent)] px-4 text-[13px] font-medium text-black transition-[filter] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
    >
      <span>{label()}</span>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <line x1="5" y1="12" x2="19" y2="12" />
        <polyline points="12 5 19 12 12 19" />
      </svg>
    </button>
  );
}

// -------- Step content --------

function Welcome() {
  return (
    <div class="flex flex-col items-center gap-5 text-center">
      <div
        class="flex h-24 w-24 items-center justify-center rounded-full"
        style={{
          background:
            "radial-gradient(circle, rgba(124,156,255,0.35), rgba(124,156,255,0.05) 70%)",
        }}
      >
        <svg
          width="56"
          height="56"
          viewBox="0 0 24 24"
          fill="none"
          stroke="var(--color-accent)"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <rect x="9" y="2" width="6" height="12" rx="3" />
          <path d="M5 10a7 7 0 0 0 14 0" />
          <line x1="12" y1="17" x2="12" y2="22" />
        </svg>
      </div>
      <div>
        <h1 class="text-[24px] font-semibold">Talk to any app.</h1>
        <p class="mt-2 max-w-[360px] text-[13px] text-[var(--color-ink-mute)]">
          Vox types what you say — in Slack, Cursor, Gmail, anywhere. Press{" "}
          <span class="keycap">⌥</span> to start.
        </p>
      </div>
    </div>
  );
}

function MicStep(props: { granted: boolean; onAllow: () => void }) {
  return (
    <div class="flex flex-col items-center gap-5 text-center">
      <h2 class="text-[20px] font-semibold">Let Vox hear you.</h2>
      <p class="max-w-[360px] text-[13px] text-[var(--color-ink-mute)]">
        Vox only captures audio while you hold the hotkey. Nothing is saved to
        disk.
      </p>
      <button
        type="button"
        onClick={props.onAllow}
        class="rounded-[8px] bg-white/10 px-4 py-2 text-[13px] hover:bg-white/15"
      >
        Allow microphone
      </button>
      <PermStatus granted={props.granted} grantedLabel="Granted" pendingLabel="Waiting…" />
    </div>
  );
}

function AccessibilityStep(props: { granted: boolean }) {
  return (
    <div class="flex flex-col items-center gap-5 text-center">
      <h2 class="text-[20px] font-semibold">Let Vox type for you.</h2>
      <p class="max-w-[400px] text-[13px] text-[var(--color-ink-mute)]">
        Vox needs Accessibility to send the finished text to the app you're
        using.
      </p>
      <button
        type="button"
        onClick={() => openAccessibilitySettings()}
        class="rounded-[8px] bg-white/10 px-4 py-2 text-[13px] hover:bg-white/15"
      >
        Open System Settings
      </button>
      <PermStatus
        granted={props.granted}
        grantedLabel="Granted — restart Vox after adding it"
        pendingLabel="Waiting (polling every 2s)…"
      />
    </div>
  );
}

function PermStatus(props: {
  granted: boolean;
  grantedLabel: string;
  pendingLabel: string;
}) {
  return (
    <div
      class="flex items-center gap-2 text-[12px]"
      classList={{
        "text-[var(--color-success)]": props.granted,
        "text-[var(--color-ink-mute)]": !props.granted,
      }}
    >
      <span>{props.granted ? "✓" : "○"}</span>
      <span>{props.granted ? props.grantedLabel : props.pendingLabel}</span>
    </div>
  );
}

function KeysStep(props: {
  s: Settings | null;
  patch: (p: Partial<Settings>) => void;
}) {
  const [groqMsg, setGroqMsg] = createSignal<string | null>(null);
  const [groqOk, setGroqOk] = createSignal<boolean | null>(null);
  const [anthroMsg, setAnthroMsg] = createSignal<string | null>(null);
  const [anthroOk, setAnthroOk] = createSignal<boolean | null>(null);

  async function testGroq() {
    const key = props.s?.groq_api_key.trim() ?? "";
    setGroqMsg("Testing…");
    setGroqOk(null);
    try {
      const r = await testGroqConnection(key);
      setGroqMsg(r.message);
      setGroqOk(r.ok);
    } catch (e) {
      setGroqMsg(String(e));
      setGroqOk(false);
    }
  }

  async function testAnthro() {
    const key = props.s?.anthropic_api_key.trim() ?? "";
    setAnthroMsg("Testing…");
    setAnthroOk(null);
    try {
      const r = await testAnthropicConnection(key);
      setAnthroMsg(r.message);
      setAnthroOk(r.ok);
    } catch (e) {
      setAnthroMsg(String(e));
      setAnthroOk(false);
    }
  }

  return (
    <div class="flex w-full max-w-[400px] flex-col gap-4">
      <h2 class="text-[20px] font-semibold">Bring your own keys.</h2>
      <p class="text-[13px] text-[var(--color-ink-mute)]">
        Vox runs on your accounts. Keys stay in your Keychain.
      </p>

      <div class="flex flex-col gap-1">
        <label class="text-[12px] font-medium">Groq (required)</label>
        <div class="flex gap-2">
          <input
            type="password"
            placeholder="gsk_…"
            class="flex-1 rounded-[6px] bg-black/30 px-3 py-2 text-[13px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s?.groq_api_key ?? ""}
            onInput={(e) => props.patch({ groq_api_key: e.currentTarget.value })}
            autocomplete="off"
            spellcheck={false}
          />
          <button
            type="button"
            onClick={testGroq}
            class="rounded-[6px] bg-white/10 px-3 py-2 text-[12px] hover:bg-white/15"
          >
            Test
          </button>
        </div>
        <Show when={groqMsg()}>
          <span
            class="text-[11px]"
            classList={{
              "text-[var(--color-success)]": groqOk() === true,
              "text-[var(--color-danger)]": groqOk() === false,
              "text-[var(--color-ink-mute)]": groqOk() === null,
            }}
          >
            {groqMsg()}
          </span>
        </Show>
      </div>

      <div class="flex flex-col gap-1">
        <label class="text-[12px] font-medium">Anthropic (optional)</label>
        <div class="flex gap-2">
          <input
            type="password"
            placeholder="sk-ant-…"
            class="flex-1 rounded-[6px] bg-black/30 px-3 py-2 text-[13px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s?.anthropic_api_key ?? ""}
            onInput={(e) =>
              props.patch({ anthropic_api_key: e.currentTarget.value })
            }
            autocomplete="off"
            spellcheck={false}
          />
          <button
            type="button"
            onClick={testAnthro}
            class="rounded-[6px] bg-white/10 px-3 py-2 text-[12px] hover:bg-white/15"
          >
            Test
          </button>
        </div>
        <Show when={anthroMsg()}>
          <span
            class="text-[11px]"
            classList={{
              "text-[var(--color-success)]": anthroOk() === true,
              "text-[var(--color-danger)]": anthroOk() === false,
              "text-[var(--color-ink-mute)]": anthroOk() === null,
            }}
          >
            {anthroMsg()}
          </span>
        </Show>
      </div>
    </div>
  );
}

function TryStep() {
  return (
    <div class="flex w-full max-w-[400px] flex-col items-center gap-4 text-center">
      <h2 class="text-[20px] font-semibold">Try it.</h2>
      <p class="text-[13px] text-[var(--color-ink-mute)]">
        Hold <span class="keycap">⌥</span> and say something. Your words will
        appear below.
      </p>
      <textarea
        placeholder="Focus here, then hold ⌥ and speak."
        class="h-24 w-full resize-none rounded-[8px] bg-black/30 px-3 py-2 text-[13px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
      />
      <p class="text-[11px] text-[var(--color-ink-faint)]">
        If nothing happens, check the tray icon for errors.
      </p>
    </div>
  );
}
