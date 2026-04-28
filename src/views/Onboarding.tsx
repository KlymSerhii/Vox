import {
  createEffect,
  createSignal,
  For,
  type JSX,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  acceptGemmaTos,
  checkPermissions,
  diagnoseHotkeys,
  downloadInferenceServer,
  downloadModel,
  finishOnboarding,
  getSettings,
  inferenceServerStatus,
  listModels,
  loadModelWarm,
  onBinaryDone,
  onBinaryError,
  onBinaryProgress,
  onModelDone,
  onModelError,
  onModelProgress,
  openAccessibilitySettings,
  recommendTier,
  requestMicrophone,
  setSettings,
  type HotkeyDiagnostic,
  type InferenceServerStatus,
  type ModelStatus,
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
  // True from the moment the user clicks "Accept and download" until
  // every chosen download finishes. Used to gate the Continue button
  // so the user can't skip ahead while bytes are still moving — but
  // the "Skip for now" exit stays open at all times so a user with a
  // dead network can still bail.
  const [downloadInFlight, setDownloadInFlight] = createSignal(false);

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
          <DownloadStep
            s={s()}
            tier={tier()}
            patch={patch}
            onInFlightChange={setDownloadInFlight}
          />
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
          downloadInFlight={downloadInFlight()}
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
  downloadInFlight: boolean;
  onNext: () => void;
  onFinish: () => void;
}) {
  const disabled = () => {
    if (props.step === 1) return !props.perms.microphone;
    // While downloads are in flight, force the user to either wait
    // (so the next step's pre-warm finds the assets) or click "Skip
    // for now" to bail explicitly. This keeps the menu-bar app from
    // flipping into a broken-on-first-use state if the user races
    // ahead while a multi-GB GGUF is still streaming.
    if (props.step === MODELS_STEP && props.downloadInFlight) return true;
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
  /// Notify the parent so it can gate the "Continue" button while
  /// large downloads are still streaming. The parent owns the gate;
  /// this step owns the truth about which IDs are expected.
  onInFlightChange: (v: boolean) => void;
}) {
  const [progress, setProgress] = createSignal<
    Record<string, { downloaded: number; total: number }>
  >({});
  const [completed, setCompleted] = createSignal<Set<string>>(new Set());
  const [started, setStarted] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);
  const [llmModels, setLlmModels] = createSignal<ModelStatus[]>([]);
  const [selectedLlmId, setSelectedLlmId] = createSignal<string | null>(null);
  // Out-of-process inference server (`llama-server`). Treated as a
  // first-class download alongside the two model downloads — the
  // user can't polish without it.
  const [binaryStatus, setBinaryStatus] =
    createSignal<InferenceServerStatus | null>(null);
  const unlistens: (() => void)[] = [];

  onMount(async () => {
    // The binary download emits the same shape on `binary:*` events as
    // models do on `model:*`, so we share the progress/completed maps.
    // The per-id key keeps them disambiguated in the UI.
    const handleProgress = (p: {
      id: string;
      bytes_downloaded: number;
      total_bytes: number;
    }) => {
      setProgress((prev) => ({
        ...prev,
        [p.id]: { downloaded: p.bytes_downloaded, total: p.total_bytes },
      }));
    };
    const handleDone = (p: { id: string }) => {
      setCompleted((prev) => {
        const n = new Set(prev);
        n.add(p.id);
        return n;
      });
    };
    const handleError = (p: { id: string; error: string }) => {
      setErr(p.error);
    };

    unlistens.push(await onModelProgress(handleProgress));
    unlistens.push(await onModelDone(handleDone));
    unlistens.push(await onModelError(handleError));
    unlistens.push(await onBinaryProgress(handleProgress));
    unlistens.push(await onBinaryDone(handleDone));
    unlistens.push(await onBinaryError(handleError));

    try {
      const [all, bin] = await Promise.all([listModels(), inferenceServerStatus()]);
      setLlmModels(all.filter((m) => m.descriptor.kind === "llm"));
      setBinaryStatus(bin);
      // If the binary is already installed (e.g. the user re-runs
      // onboarding from Settings), pre-mark it as completed so the
      // gate doesn't block forever.
      if (bin.installed) {
        setCompleted((prev) => {
          const n = new Set(prev);
          n.add(bin.id);
          return n;
        });
      }
    } catch (e) {
      console.warn("listModels/inferenceServerStatus failed", e);
    }
  });

  // Default the radio selection to the recommender's preferred LLM id once
  // both inputs are available. Effect re-runs if either changes.
  createEffect(() => {
    const t = props.tier;
    const llms = llmModels();
    if (t && llms.length > 0 && !selectedLlmId()) {
      setSelectedLlmId(t.llm_model_id);
    }
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
  });

  const selectedDescriptor = () => {
    const id = selectedLlmId();
    if (!id) return null;
    return llmModels().find((m) => m.descriptor.id === id)?.descriptor ?? null;
  };

  async function start() {
    const t = props.tier;
    const cur = props.s;
    const llmId = selectedLlmId();
    if (!t || !cur || !llmId) return;
    setStarted(true);
    if (!cur.gemma_tos_accepted) {
      await acceptGemmaTos();
    }
    await props.patch({
      stt_engine: "whisper-local",
      stt_model_id: t.stt_model_id,
      llm_engine: "gemma-local",
      llm_model_id: llmId,
      gemma_tos_accepted: true,
    });
    try {
      props.onInFlightChange(true);
      await Promise.all([
        downloadModel(t.stt_model_id),
        downloadModel(llmId),
        // Pull the `llama-server` binary alongside the GGUFs — the
        // three downloads run concurrently so total wall-clock is
        // dominated by the largest (Gemma) rather than the sum.
        downloadInferenceServer(),
      ]);
      // Pre-warm so first dictation doesn't pay the cold-load tax.
      // We don't pre-warm the inference server here — that would
      // start `llama-server` immediately, but the user might not
      // dictate for a while, so we defer until the first polish.
      await Promise.allSettled([
        loadModelWarm(t.stt_model_id),
        loadModelWarm(llmId),
      ]);
    } catch (e) {
      setErr(String(e));
    } finally {
      props.onInFlightChange(false);
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
        <Show when={llmModels().length > 0 && props.tier}>
          {(t) => (
            <div class="flex flex-col gap-2" role="radiogroup" aria-label="Gemma 4 model">
              <div class="flex items-baseline justify-between">
                <span class="text-[13px] font-medium">Choose your Gemma 4 model</span>
                <Show when={t().physical_ram_gb > 0}>
                  <span class="text-[11px] text-[var(--color-ink-faint)]">
                    Your Mac has {t().physical_ram_gb} GB RAM
                  </span>
                </Show>
              </div>
              <For each={llmModels()}>
                {(m) => (
                  <LlmCard
                    model={m}
                    selected={selectedLlmId() === m.descriptor.id}
                    recommended={m.descriptor.tier === t().tier}
                    swapWarn={
                      t().physical_ram_gb > 0 &&
                      m.descriptor.ram_peak_mb >
                        t().physical_ram_gb * 1024 * 0.85
                    }
                    onSelect={() => setSelectedLlmId(m.descriptor.id)}
                  />
                )}
              </For>
            </div>
          )}
        </Show>
        <button
          type="button"
          class="self-start rounded-[8px] bg-[var(--color-accent)] px-4 py-2 text-[13px] font-medium text-black hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={!selectedLlmId() || !props.tier}
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
            <Show when={selectedDescriptor()} fallback={null}>
              {(d) => (
                <DownloadBar
                  label={shortLlmName(d().name)}
                  done={completed().has(d().id)}
                  prog={progress()[d().id]}
                />
              )}
            </Show>
            <Show when={binaryStatus()}>
              {(b) => (
                <DownloadBar
                  label="Inference engine"
                  done={completed().has(b().id)}
                  prog={progress()[b().id]}
                />
              )}
            </Show>
          </div>
        )}
      </Show>
      <Show when={err()}>
        <p class="text-[12px] text-[var(--color-danger)]">{err()}</p>
      </Show>
    </div>
  );
}

function LlmCard(props: {
  model: ModelStatus;
  selected: boolean;
  recommended: boolean;
  swapWarn: boolean;
  onSelect: () => void;
}) {
  const d = () => props.model.descriptor;
  const displayName = () => d().name.replace(/\s*\([^)]*\)/g, "");
  const sizeGb = () => (d().size_bytes / 1e9).toFixed(1);
  const ramGb = () => (d().ram_peak_mb / 1024).toFixed(0);
  return (
    <button
      type="button"
      role="radio"
      aria-checked={props.selected}
      onClick={props.onSelect}
      class="flex flex-col gap-1 rounded-[10px] bg-[var(--color-surface-raised)] px-3 py-2.5 text-left transition-[box-shadow,background-color]"
      classList={{
        "ring-1 ring-[var(--color-border)]": !props.selected,
        "ring-2 ring-[var(--color-accent)]": props.selected,
      }}
    >
      <div class="flex items-center justify-between gap-2">
        <span class="text-[13px] font-medium text-[var(--color-ink)]">
          {displayName()}
        </span>
        <span
          class="flex h-3.5 w-3.5 items-center justify-center rounded-full"
          classList={{
            "ring-1 ring-[var(--color-border-strong)]": !props.selected,
            "bg-[var(--color-accent)]": props.selected,
          }}
          aria-hidden="true"
        >
          <Show when={props.selected}>
            <span class="h-1.5 w-1.5 rounded-full bg-black" />
          </Show>
        </span>
      </div>
      <span class="text-[11px] text-[var(--color-ink-mute)]">
        ~{sizeGb()} GB on disk · ~{ramGb()} GB peak RAM
      </span>
      <Show when={props.recommended || props.swapWarn}>
        <div class="mt-1 flex flex-wrap gap-1">
          <Show when={props.recommended}>
            <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
              Recommended
            </span>
          </Show>
          <Show when={props.swapWarn}>
            <span class="rounded-full bg-[var(--color-danger)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-danger)]">
              May swap on this Mac
            </span>
          </Show>
        </div>
      </Show>
    </button>
  );
}

function shortLlmName(name: string): string {
  return name.replace(/\s*\([^)]*\)/g, "").replace(/ —.*$/, "");
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
