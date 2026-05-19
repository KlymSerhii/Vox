import {
  createSignal,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  checkPermissions,
  finishOnboarding,
  getSettings,
  parakeetAvailability,
  recommendTier,
  requestMicrophone,
  setSettings,
  type ParakeetAvailability,
  type PermissionStatus,
  type Settings,
  type TierRecommendation,
} from "../lib/ipc";
import { PrimaryButton } from "./onboarding/components/PrimaryButton";
import { Progress } from "./onboarding/components/Progress";
import {
  LANGUAGE_STEP,
  LAST_STEP,
  MIC_STEP,
  MODELS_STEP,
  TIPS_STEP,
  type Step,
} from "./onboarding/constants";
import { DownloadStep } from "./onboarding/steps/DownloadStep";
import { HowToUseStep } from "./onboarding/steps/HowToUseStep";
import { LanguageStep } from "./onboarding/steps/LanguageStep";
import { MicStep } from "./onboarding/steps/MicStep";
import { TipsStep } from "./onboarding/steps/TipsStep";
import { Welcome } from "./onboarding/steps/WelcomeStep";

export default function OnboardingView() {
  const [step, setStep] = createSignal<Step>(0);
  const [s, setS] = createSignal<Settings | null>(null);
  const [tier, setTier] = createSignal<TierRecommendation | null>(null);
  const [perms, setPerms] = createSignal<PermissionStatus>({
    microphone: false,
    accessibility: false,
  });
  // Probed once on mount; the language step uses it for engine routing
  // and to decide whether the "auto-detect → Parakeet" path is even
  // viable on this Mac.
  const [parakeet, setParakeet] = createSignal<ParakeetAvailability | null>(
    null,
  );
  // Gates Continue while downloads run, AND until the chosen engine's
  // required artifacts are on disk. The user cannot skip past the models
  // step: Whisper is mandatory, plus Gemma + llama-server when the
  // Gemma engine is chosen.
  const [downloadInFlight, setDownloadInFlight] = createSignal(false);
  const [modelsReady, setModelsReady] = createSignal(false);

  let pollTimer: ReturnType<typeof setInterval> | null = null;

  onMount(async () => {
    try {
      const settings = await getSettings();
      setS(settings);
      setTier(await recommendTier());
    } catch (e) {
      console.error(e);
    }
    try {
      setParakeet(await parakeetAvailability());
    } catch (e) {
      console.warn("parakeetAvailability failed:", e);
    }
    const poll = async () => {
      try {
        setPerms(await checkPermissions());
      } catch {}
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
    try {
      await finishOnboarding();
    } catch (err) {
      console.error("[onboarding] finish: backend failed", err);
    }
    // Close from the frontend too — backend close() can race against a
    // lost window handle and leave the wizard stuck open.
    try {
      await getCurrentWindow().close();
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

  /// Skip handler runs the same step advance, plus a per-step reconcile
  /// pass. On the Language step we have to make sure the persisted
  /// stt_engine matches what's actually available on this Mac:
  /// `auto`/`en` would otherwise leave the user with `parakeet-local`
  /// queued from a default while the runtime can't load it. Falls back
  /// to `whisper-local` when parakeet probe says unavailable so the
  /// downloads step doesn't try to fetch a Parakeet model that won't
  /// run.
  async function skip() {
    if (step() === LANGUAGE_STEP) {
      const cur = s();
      const pk = parakeet();
      if (cur && pk && !pk.available && cur.stt_engine !== "whisper-local") {
        try {
          await patch({ stt_engine: "whisper-local" });
        } catch (err) {
          console.warn("language skip: reconcile failed", err);
        }
      }
    }
    next();
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

      {/* Scroll lives on `main` so its scrollbar tracks the window
          edge instead of slicing through the content card. The inner
          wrapper uses auto block-margins so short steps stay centered
          while the long models step scrolls. `min-h-0` lets `flex-1`
          actually constrain main's height (default flex children
          can't shrink below content). */}
      <main class="flex flex-1 flex-col items-center px-10 min-h-0 overflow-y-auto">
        <div class="my-auto flex w-full flex-col items-center py-6">
          <Show when={step() === 0}>
            <Welcome />
          </Show>
          <Show when={step() === LANGUAGE_STEP}>
            <LanguageStep
              s={s()}
              parakeet={parakeet()}
              patch={patch}
            />
          </Show>
          <Show when={step() === MIC_STEP}>
            <MicStep
              granted={perms().microphone}
              onAllow={async () => {
                await requestMicrophone();
                setPerms(await checkPermissions());
              }}
            />
          </Show>
          <Show when={step() === MODELS_STEP}>
            <DownloadStep
              s={s()}
              tier={tier()}
              patch={patch}
              onInFlightChange={setDownloadInFlight}
              onReadyChange={setModelsReady}
            />
          </Show>
          <Show when={step() === TIPS_STEP}>
            <TipsStep />
          </Show>
          <Show when={step() === LAST_STEP}>
            <HowToUseStep s={s()} />
          </Show>
        </div>
      </main>

      <footer class="flex items-center justify-between border-t border-[var(--color-border)] px-6 py-4">
        <Show
          when={step() !== LAST_STEP}
          fallback={<span aria-hidden="true" />}
        >
          {/* The models step is gated — no Skip path. Other optional
              steps keep the soft Skip link. */}
          <Show
            when={step() !== 0 && step() !== MODELS_STEP}
            fallback={<span aria-hidden="true" />}
          >
            <button
              type="button"
              class="text-[12px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
              onClick={() => void skip()}
            >
              Skip
            </button>
          </Show>
        </Show>
        <PrimaryButton
          step={step()}
          perms={perms()}
          s={s()}
          downloadInFlight={downloadInFlight()}
          modelsReady={modelsReady()}
          onNext={next}
          onFinish={handleFinish}
        />
      </footer>
    </div>
  );
}
