import { createSignal, For, type JSX, onCleanup, onMount, Show } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  acceptGemmaTos,
  checkPermissions,
  diagnoseHotkeys,
  downloadModel,
  finishOnboarding,
  getSettings,
  loadModelWarm,
  onModelDone,
  onModelError,
  onModelProgress,
  openAccessibilitySettings,
  recommendTier,
  requestMicrophone,
  setSettings,
  type HotkeyDiagnostic,
  type PermissionStatus,
  type Settings,
  type TierRecommendation,
} from "../lib/ipc";

// 2026-04-26: cloud onboarding (StackStep + KeysStep) was removed when
// Vox went fully on-device. The flow is now linear: Welcome → Mic →
// Accessibility → Local model download → Try it / How to use.
type Step = 0 | 1 | 2 | 3 | 4;

const STEP_COUNT = 5;
const LAST_STEP: Step = 4;
const MODELS_STEP: Step = 3;

/**
 * Render a Tauri global-shortcut spec like "Alt+Shift+V" or "F13" as a
 * compact glyph string, e.g. "⌥⇧V" / "F13".
 *
 * - Bare modifier IDs like `AltRight` / `AltLeft` are rendered as `⌥`
 *   so the keycap row reads naturally even when the user has chosen
 *   Right-Option (which we now warn about separately because it's
 *   unreliable through tauri-plugin-global-shortcut on macOS).
 * - The Tauri spec `"CommandOrControl+Shift+Space"` is collapsed to
 *   `⌘⇧Space` rather than `CommandOrControl⇧Space`.
 * - Other shortcuts are split on `+` and each token mapped through the
 *   modifier table; unknown tokens fall through verbatim.
 */
export function formatHotkey(spec: string | null | undefined): string {
  if (!spec) return "";
  const trimmed = spec.trim();
  if (!trimmed) return "";
  if (trimmed === "AltRight" || trimmed === "AltLeft") return "⌥";
  const MODS: Record<string, string> = {
    Alt: "⌥",
    Option: "⌥",
    Shift: "⇧",
    Cmd: "⌘",
    Command: "⌘",
    Meta: "⌘",
    Super: "⌘",
    Ctrl: "⌃",
    Control: "⌃",
    CommandOrControl: "⌘",
    Semicolon: ";",
  };
  return trimmed
    .split("+")
    .map((tok) => {
      const t = tok.trim();
      if (!t) return "";
      return MODS[t] ?? t;
    })
    .join("");
}

export default function OnboardingView() {
  const [step, setStep] = createSignal<Step>(0);
  const [s, setS] = createSignal<Settings | null>(null);
  const [tier, setTier] = createSignal<TierRecommendation | null>(null);
  const [perms, setPerms] = createSignal<PermissionStatus>({
    microphone: false,
    accessibility: false,
  });

  let pollTimer: ReturnType<typeof setInterval> | null = null;

  onMount(async () => {
    try {
      const settings = await getSettings();
      setS(settings);
      setTier(await recommendTier());
    } catch (e) {
      console.error(e);
    }
    const poll = async () => {
      try {
        setPerms(await checkPermissions());
      } catch {
        /* non-fatal */
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
    console.info("[onboarding] finish: click registered");
    try {
      await finishOnboarding();
      console.info("[onboarding] finish: backend ack");
    } catch (err) {
      console.error("[onboarding] finish: backend failed", err);
    }
    // Close from the frontend regardless. Backend already calls close()
    // too, but if the IPC errored or the window handle was lost, this
    // guarantees the user gets out of the wizard.
    try {
      await getCurrentWindow().close();
      console.info("[onboarding] finish: window closed");
    } catch (err) {
      console.error("[onboarding] finish: close failed", err);
    }
  }

  function next() {
    setStep(Math.min(LAST_STEP, step() + 1) as Step);
  }

  function back() {
    setStep(Math.max(0, step() - 1) as Step);
  }

  return (
    <div class="flex h-screen w-screen flex-col bg-[var(--color-bg)] text-[var(--color-ink)]">
      <header class="flex items-center justify-between px-6 pt-5 pb-2">
        <Show when={step() > 0} fallback={<div class="h-6 w-6" />}>
          <button
            type="button"
            onClick={back}
            class="rounded p-1 text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]"
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
        <Show when={step() === MODELS_STEP}>
          <DownloadStep s={s()} tier={tier()} patch={patch} />
        </Show>
        <Show when={step() === LAST_STEP}>
          <HowToUseStep s={s()} />
        </Show>
      </main>

      <footer class="flex items-center justify-between border-t border-[var(--color-border)] px-6 py-4">
        <Show
          when={step() !== LAST_STEP}
          fallback={<span aria-hidden="true" />}
        >
          <button
            type="button"
            class="text-[12px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
            onClick={next}
          >
            {step() === 0 ? "" : step() === MODELS_STEP ? "Skip for now" : "Skip"}
          </button>
        </Show>
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
      <For each={Array.from({ length: STEP_COUNT }, (_, i) => i)}>
        {(i) => (
          <span
            class="h-1.5 w-1.5 rounded-full transition-colors"
            classList={{
              "bg-[var(--color-accent)]": props.step === i,
              "bg-[var(--color-border-strong)]": props.step !== i,
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
  const disabled = () => {
    if (props.step === 1) return !props.perms.microphone;
    return false;
  };
  const label = () => {
    if (props.step === 0) return "Get started";
    if (props.step === LAST_STEP) return "Finish";
    return "Continue";
  };
  const isFinish = () => props.step === LAST_STEP;
  return (
    <button
      type="button"
      disabled={disabled()}
      onClick={() => {
        console.info("[onboarding] PrimaryButton click", { step: props.step, isFinish: isFinish() });
        if (isFinish()) {
          props.onFinish();
        } else {
          props.onNext();
        }
      }}
      class="inline-flex h-9 items-center gap-2 rounded-[8px] bg-[var(--color-accent)] px-4 text-[13px] font-medium text-black transition-[filter] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
    >
      <Show
        when={isFinish()}
        fallback={
          <>
            <span>{label()}</span>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="5" y1="12" x2="19" y2="12" />
              <polyline points="12 5 19 12 12 19" />
            </svg>
          </>
        }
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="20 6 9 17 4 12" />
        </svg>
        <span>{label()}</span>
      </Show>
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
          Vox runs offline on your Mac.
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
        Vox only captures audio while you pressed the hotkey. Nothing is saved
        to disk.
      </p>
      <button
        type="button"
        onClick={props.onAllow}
        class="rounded-[8px] bg-[var(--color-control)] px-4 py-2 text-[13px] hover:bg-[var(--color-control-hover)]"
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
        class="rounded-[8px] bg-[var(--color-control)] px-4 py-2 text-[13px] hover:bg-[var(--color-control-hover)]"
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

// 2026-04-26: StackStep + KeysStep were removed when cloud engines went
// away. Onboarding is now a fixed local flow: Welcome → Mic →
// Accessibility → DownloadStep → HowToUseStep.

function DownloadStep(props: {
  s: Settings | null;
  tier: TierRecommendation | null;
  patch: (p: Partial<Settings>) => void;
}) {
  const [progress, setProgress] = createSignal<
    Record<string, { downloaded: number; total: number }>
  >({});
  const [completed, setCompleted] = createSignal<Set<string>>(new Set());
  const [started, setStarted] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);
  const unlistens: (() => void)[] = [];

  onMount(async () => {
    unlistens.push(
      await onModelProgress((p) => {
        setProgress((prev) => ({
          ...prev,
          [p.id]: { downloaded: p.bytes_downloaded, total: p.total_bytes },
        }));
      }),
    );
    unlistens.push(
      await onModelDone((p) => {
        setCompleted((prev) => {
          const n = new Set(prev);
          n.add(p.id);
          return n;
        });
      }),
    );
    unlistens.push(
      await onModelError((p) => {
        setErr(p.error);
      }),
    );
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
  });

  async function start() {
    const t = props.tier;
    const cur = props.s;
    if (!t || !cur) return;
    setStarted(true);
    if (!cur.gemma_tos_accepted) {
      await acceptGemmaTos();
    }
    await props.patch({
      stt_engine: "whisper-local",
      stt_model_id: t.stt_model_id,
      llm_engine: "gemma-local",
      llm_model_id: t.llm_model_id,
      gemma_tos_accepted: true,
    });
    try {
      await Promise.all([
        downloadModel(t.stt_model_id),
        downloadModel(t.llm_model_id),
      ]);
      // Pre-warm so first dictation doesn't pay the cold-load tax.
      await Promise.allSettled([
        loadModelWarm(t.stt_model_id),
        loadModelWarm(t.llm_model_id),
      ]);
    } catch (e) {
      setErr(String(e));
    }
  }

  return (
    <div class="flex w-full max-w-[420px] flex-col gap-4">
      <h2 class="text-[20px] font-semibold">Set up local models.</h2>
      <Show when={!started()}>
        <p class="text-[13px] text-[var(--color-ink-mute)]">
          Vox will download Whisper and Gemma 4 from Hugging Face. By
          continuing you accept the{" "}
          <a
            class="underline"
            href="https://ai.google.dev/gemma/terms"
            target="_blank"
          >
            Gemma Terms of Use
          </a>
          .
        </p>
        <button
          type="button"
          class="self-start rounded-[8px] bg-[var(--color-accent)] px-4 py-2 text-[13px] font-medium text-black hover:brightness-110"
          onClick={start}
        >
          Accept and download
        </button>
      </Show>
      <Show when={started() && props.tier}>
        {(t) => (
          <div class="space-y-3">
            <DownloadBar
              label="Whisper"
              done={completed().has(t().stt_model_id)}
              prog={progress()[t().stt_model_id]}
            />
            <DownloadBar
              label="Gemma"
              done={completed().has(t().llm_model_id)}
              prog={progress()[t().llm_model_id]}
            />
          </div>
        )}
      </Show>
      <Show when={err()}>
        <p class="text-[12px] text-[var(--color-danger)]">{err()}</p>
      </Show>
    </div>
  );
}

function DownloadBar(props: {
  label: string;
  done: boolean;
  prog?: { downloaded: number; total: number };
}) {
  const pct = () => {
    if (props.done) return 100;
    if (!props.prog) return 0;
    if (props.prog.total <= 0) return 0;
    return Math.min(100, (props.prog.downloaded / props.prog.total) * 100);
  };
  return (
    <div>
      <div class="mb-1 flex justify-between text-[11px] text-[var(--color-ink-mute)]">
        <span>{props.label}</span>
        <span>{props.done ? "Done" : `${pct().toFixed(0)}%`}</span>
      </div>
      <div class="h-1.5 w-full overflow-hidden rounded-full bg-[var(--color-control)]">
        <div
          class="h-full rounded-full bg-[var(--color-accent)] transition-[width]"
          style={{ width: `${pct()}%` }}
        />
      </div>
    </div>
  );
}

// 2026-04-26: KeysStep + TryStep deleted — no cloud secrets, and the
// cloud path that rendered TryStep as a model-step fallback is gone.

function HowToUseStep(props: { s: Settings | null }) {
  // Always render the BOUND hotkey (what the OS will actually fire),
  // not the configured one — they can differ when the configured key
  // wasn't registerable and the runtime fell back. We fetch the active
  // bindings on mount so the keycaps read the truth even on a fresh
  // install where settings haven't been touched.
  const [diags, setDiags] = createSignal<Map<string, HotkeyDiagnostic>>(
    new Map(),
  );

  onMount(async () => {
    try {
      const d = await diagnoseHotkeys();
      const map = new Map<string, HotkeyDiagnostic>();
      for (const e of d.entries) map.set(e.purpose, e);
      setDiags(map);
    } catch (err) {
      console.warn("diagnoseHotkeys failed:", err);
    }
  });

  const ptt = () => diags().get("push-to-talk");
  const cycle = () => diags().get("mode-cycle");

  const pttGlyph = () =>
    formatHotkey(ptt()?.bound) ||
    formatHotkey(props.s?.hotkey) ||
    "⌘⌥L";
  const cycleGlyph = () =>
    formatHotkey(cycle()?.bound) ||
    formatHotkey(props.s?.mode_cycle_hotkey) ||
    "⌥M";

  const handsFree = () => props.s?.hands_free_enabled ?? true;
  const pttVerb = () => (handsFree() ? "Press" : "Hold");
  const pttTail = () =>
    handsFree()
      ? "to start dictating in any text field. Press again to stop."
      : "to start dictating in any text field. Release when done.";

  return (
    <div class="flex w-full max-w-[420px] flex-col gap-5">
      <div class="flex flex-col gap-2">
        <h2 class="text-[24px] font-semibold leading-tight">
          You're ready.
        </h2>
        <p class="text-[13px] text-[var(--color-ink-mute)]">
          Vox lives in your menu bar. Click the icon any time to see status,
          switch modes, or open settings.
        </p>
      </div>

      <ul class="flex flex-col gap-3">
        <TipRow
          label={<>{pttVerb()} <span class="keycap">{pttGlyph()}</span></>}
          description={
            pttFailed(ptt())
              ? "— Vox can't listen yet, grant Accessibility to enable."
              : pttTail()
          }
          fellBackNote={fellBackNote(ptt())}
          errorAction={
            pttFailed(ptt())
              ? () => void openAccessibilitySettings()
              : undefined
          }
          errorActionLabel={pttFailed(ptt()) ? "Grant Accessibility" : undefined}
        />
        <TipRow
          label={<span class="keycap">{cycleGlyph()}</span>}
          description="to cycle Voice Modes. Match your tone to the app you're in."
          fellBackNote={fellBackNote(cycle())}
        />
      </ul>

      <p class="text-[11px] text-[var(--color-ink-faint)]">
        Need to change anything? Open Settings from the menu bar icon.
        Welcome to Vox.
      </p>
    </div>
  );
}

function fellBackNote(d: HotkeyDiagnostic | undefined): string | null {
  if (!d || !d.fell_back || !d.bound) return null;
  const cfg = formatHotkey(d.configured) || d.configured;
  const bnd = formatHotkey(d.bound) || d.bound;
  return `Configured ${cfg} wasn't available on your Mac — Vox is using ${bnd}.`;
}

/** True if push-to-talk diagnostic exists, has no binding, and the user
 * has a non-empty configured hotkey. The "You're ready" view uses this
 * to swap in actionable copy ("grant Accessibility") in place of the
 * normal "to start dictating" hint. */
function pttFailed(d: HotkeyDiagnostic | undefined): boolean {
  if (!d) return false;
  return !d.bound && d.configured !== "";
}

function TipRow(props: {
  label: JSX.Element;
  description: string;
  fellBackNote?: string | null;
  errorAction?: (() => void) | undefined;
  errorActionLabel?: string | undefined;
}) {
  return (
    <li
      class="flex flex-col gap-1 rounded-[14px] bg-[var(--color-surface-raised)] px-3 py-2.5 ring-1 ring-[var(--color-border)]"
      style={{
        "box-shadow":
          "inset 0 1px 0 var(--glass-highlight), 0 1px 2px var(--glass-shadow-soft)",
      }}
    >
      <div class="flex items-baseline gap-2">
        <span class="text-[13px] font-medium text-[var(--color-ink)]">
          {props.label}
        </span>
        <span
          class="text-[13px]"
          classList={{
            "text-[var(--color-ink-mute)]": !props.errorAction,
            "text-[var(--color-danger)]": !!props.errorAction,
          }}
        >
          {props.description}
        </span>
      </div>
      <Show when={props.fellBackNote}>
        <span class="text-[11px] text-[var(--color-ink-faint)]">
          {props.fellBackNote}
        </span>
      </Show>
      <Show when={props.errorAction && props.errorActionLabel}>
        <button
          type="button"
          onClick={props.errorAction}
          class="self-start mt-1 rounded-[6px] bg-[var(--color-danger)]/15 px-2 py-1 text-[11px] text-[var(--color-danger)] hover:bg-[var(--color-danger)]/25"
        >
          {props.errorActionLabel}
        </button>
      </Show>
    </li>
  );
}
