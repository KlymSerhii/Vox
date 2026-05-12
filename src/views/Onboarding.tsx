import {
  createEffect,
  createMemo,
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
  appleFmAvailability,
  checkPermissions,
  diagnoseHotkeys,
  downloadInferenceServer,
  downloadModel,
  finishOnboarding,
  getSettings,
  inferenceServerStatus,
  languageDisplayName,
  listModels,
  loadModelWarm,
  onBinaryDone,
  onBinaryError,
  onBinaryProgress,
  onModelDone,
  onModelError,
  onModelProgress,
  openAccessibilitySettings,
  openExternalUrl,
  PARAKEET_SUPPORTED_LANGUAGES,
  parakeetAvailability,
  parakeetSupportsLanguage,
  recommendedSttEngineFor,
  recommendTier,
  requestMicrophone,
  setSettings,
  type AppleFmAvailability,
  type HotkeyDiagnostic,
  type InferenceServerStatus,
  type ModelStatus,
  type ParakeetAvailability,
  type PermissionStatus,
  type Settings,
  type TierRecommendation,
} from "../lib/ipc";
import { VoxLogo } from "../components/VoxLogo";

type PolishChoice = "apple-fm" | "gemma-local";

type Step = 0 | 1 | 2 | 3 | 4;

const STEP_COUNT = 5;
const LAST_STEP: Step = 4;
const MODELS_STEP: Step = 3;
const MIC_STEP: Step = 2;
const LANGUAGE_STEP: Step = 1;

// Common languages that aren't in Parakeet's 25-language set. Listed
// here so the language picker can offer them as a one-tap "this routes
// to Whisper" path without forcing the user into a free-form input.
const NON_PARAKEET_COMMON: readonly string[] = [
  "zh", "ja", "ko", "ar", "hi", "tr", "vi", "th",
] as const;

/** Render a Tauri global-shortcut spec ("Alt+Shift+V", "F13") as a glyph
 *  string ("⌥⇧V", "F13"). Bare `AltRight`/`AltLeft` map to `⌥`.
 *
 *  When the trailing key is one whose shifted glyph is more readable
 *  than `Shift+<key>` (e.g. `Slash` → `?`, `Semicolon` → `:`), and
 *  `Shift` is in the modifier set, the rendering collapses `Shift+key`
 *  to the shifted glyph alone. This matches macOS menu conventions —
 *  `Cmd+Shift+/` is shown as `⌘?`, not `⌘⇧/`. */
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
  };
  const KEYS_UNSHIFTED: Record<string, string> = {
    Slash: "/",
    Semicolon: ";",
    Backslash: "\\",
    Comma: ",",
    Period: ".",
    Quote: "'",
  };
  const KEYS_SHIFTED: Record<string, string> = {
    Slash: "?",
    Semicolon: ":",
    Backslash: "|",
    Comma: "<",
    Period: ">",
    Quote: "\"",
  };
  const tokens = trimmed.split("+").map((t) => t.trim());
  const last = tokens[tokens.length - 1];
  const hasShift = tokens.includes("Shift");
  if (hasShift && KEYS_SHIFTED[last]) {
    const head = tokens
      .slice(0, -1)
      .filter((t) => t !== "Shift")
      .map((t) => MODS[t] ?? t);
    return head.join("") + KEYS_SHIFTED[last];
  }
  return tokens
    .map((t) => MODS[t] ?? KEYS_UNSHIFTED[t] ?? t)
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
  modelsReady: boolean;
  onNext: () => void;
  onFinish: () => void;
}) {
  // Gate Continue on mic step until granted, and on models step until
  // every required artifact is on disk for the chosen engine. Racing
  // ahead with a partial GGUF leaves the app broken on first use.
  const disabled = () =>
    (props.step === MIC_STEP && !props.perms.microphone) ||
    (props.step === MODELS_STEP &&
      (props.downloadInFlight || !props.modelsReady));
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
      onClick={() => (isFinish() ? props.onFinish() : props.onNext())}
      class="inline-flex h-9 items-center gap-2 rounded-[8px] bg-[var(--color-accent)] px-4 text-[13px] font-medium text-[var(--color-on-accent)] transition-[filter] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
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

function Welcome() {
  return (
    <div class="flex flex-col items-center gap-5 text-center">
      <VoxLogo size={96} />
      <div>
        <h1 class="text-[24px] font-semibold">On-device dictation.</h1>
        <p class="mt-2 max-w-[360px] text-[13px] text-[var(--color-ink-mute)]">
          Press, speak, paste. Vox runs entirely on this Mac — nothing
          leaves the device.
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
      <Show
        when={props.granted}
        fallback={
          <svg
            width="12"
            height="12"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <circle cx="12" cy="12" r="9" />
          </svg>
        }
      >
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <polyline points="4 12 10 18 20 6" />
        </svg>
      </Show>
      <span>{props.granted ? props.grantedLabel : props.pendingLabel}</span>
    </div>
  );
}

// Sentinel for the secondary `<select>` so the user can opt into the
// Parakeet auto-detect path even after first picking an explicit code.
const AUTO_VALUE = "__auto__";

function LanguageStep(props: {
  s: Settings | null;
  parakeet: ParakeetAvailability | null;
  patch: (p: Partial<Settings>) => void;
}) {
  // Render the 25 supported codes in display-name order so e.g. "Czech"
  // is near "Croatian" instead of being grouped by ISO ordering.
  const supportedSorted = createMemo(() =>
    [...PARAKEET_SUPPORTED_LANGUAGES].sort((a, b) =>
      languageDisplayName(a).localeCompare(languageDisplayName(b)),
    ),
  );
  const otherSorted = createMemo(() =>
    [...NON_PARAKEET_COMMON].sort((a, b) =>
      languageDisplayName(a).localeCompare(languageDisplayName(b)),
    ),
  );

  // Buckets the current language for the radio cards: `auto`, an
  // explicit Parakeet code, or "other" (which collapses every
  // non-Parakeet code into a single card backed by the secondary
  // dropdown).
  type Bucket = "auto" | "supported" | "other";
  const bucket = (): Bucket => {
    const cur = props.s?.language ?? "auto";
    if (cur === "auto") return "auto";
    if (parakeetSupportsLanguage(cur)) return "supported";
    return "other";
  };

  // Tracks the dropdown's last explicit choice so flipping back to the
  // "Auto" card doesn't lose the user's prior pick. Initialised from
  // the persisted setting when it's an explicit Parakeet code.
  const [supportedPick, setSupportedPick] = createSignal<string>(
    props.s?.language && parakeetSupportsLanguage(props.s.language)
      ? props.s.language
      : "en",
  );
  const [otherPick, setOtherPick] = createSignal<string>(
    props.s?.language && !parakeetSupportsLanguage(props.s.language) && props.s.language !== "auto"
      ? props.s.language
      : "zh",
  );

  const parakeetOk = () => props.parakeet?.available ?? false;

  async function persist(language: string) {
    const engine = recommendedSttEngineFor(language, parakeetOk());
    const stt_model_id =
      engine === "parakeet-local"
        ? "parakeet-tdt-0.6b-v3"
        : (props.s?.stt_model_id ?? "");
    await props.patch({ language, stt_engine: engine, stt_model_id });
  }

  // The Auto card highlights itself by default and labels itself
  // "Parakeet" when the probe succeeds, but a fresh user lands here
  // with the persisted `stt_engine` still at its `whisper-local`
  // default — so a Continue click without ever touching a card leaves
  // the models step filtering Whisper-only. Reconcile the persisted
  // engine + model id with what this step's UI implies. No-ops once
  // aligned; re-runs only when the language or parakeet probe changes.
  createEffect(() => {
    const cur = props.s;
    const pk = props.parakeet;
    if (!cur || !pk) return;
    const expectedEngine = recommendedSttEngineFor(cur.language, pk.available);
    if (cur.stt_engine !== expectedEngine) {
      void persist(cur.language);
    }
  });

  async function pickAuto() {
    await persist("auto");
  }
  async function pickSupported(code: string) {
    setSupportedPick(code);
    await persist(code);
  }
  async function pickOther(code: string) {
    setOtherPick(code);
    await persist(code);
  }

  const engineForBucket = (b: Bucket): "parakeet-local" | "whisper-local" => {
    if (!parakeetOk()) return "whisper-local";
    return b === "other" ? "whisper-local" : "parakeet-local";
  };

  return (
    <div class="flex w-full max-w-[440px] flex-col gap-4">
      <div class="flex flex-col gap-1.5">
        <h2 class="text-[20px] font-semibold">What language do you speak?</h2>
        <p class="text-[13px] text-[var(--color-ink-mute)]">
          We'll pick the best on-device model for your language. You can
          change this later in Settings.
        </p>
      </div>

      <div
        class="flex flex-col gap-2"
        role="radiogroup"
        aria-label="Dictation language"
      >
        <LanguageCard
          title="Detect automatically"
          subtitle={
            parakeetOk()
              ? "Parakeet — fast multilingual recognition across 25 supported languages."
              : "Whisper picks the language automatically."
          }
          recommended={true}
          engine={engineForBucket("auto")}
          selected={bucket() === "auto"}
          onSelect={pickAuto}
        />

        <LanguageCard
          title="Pick a specific language"
          subtitle="Tell Vox exactly which language you'll speak in."
          recommended={false}
          engine={engineForBucket("supported")}
          selected={bucket() === "supported"}
          onSelect={() => pickSupported(supportedPick())}
        >
          <select
            aria-label="Specific language"
            value={supportedPick()}
            onChange={(e) => pickSupported(e.currentTarget.value)}
            class="mt-2 w-full rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] text-[var(--color-ink)] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
          >
            <For each={supportedSorted()}>
              {(code) => (
                <option value={code}>
                  {languageDisplayName(code)} ({code})
                </option>
              )}
            </For>
          </select>
        </LanguageCard>

        <LanguageCard
          title="Other language (uses Whisper)"
          subtitle="For languages outside Parakeet's 25-language set."
          recommended={false}
          engine="whisper-local"
          selected={bucket() === "other"}
          onSelect={() => pickOther(otherPick())}
        >
          <select
            aria-label="Other language"
            value={otherPick() === "auto" ? AUTO_VALUE : otherPick()}
            onChange={(e) => {
              const v = e.currentTarget.value;
              pickOther(v === AUTO_VALUE ? "auto" : v);
            }}
            class="mt-2 w-full rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] text-[var(--color-ink)] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
          >
            <For each={otherSorted()}>
              {(code) => (
                <option value={code}>
                  {languageDisplayName(code)} ({code})
                </option>
              )}
            </For>
            <option value={AUTO_VALUE}>Other / Multilingual (auto-detect)</option>
          </select>
        </LanguageCard>
      </div>

      <Show when={!parakeetOk() && props.parakeet}>
        <p class="text-[11px] text-[var(--color-ink-faint)]">
          Parakeet isn't available on this Mac
          {props.parakeet!.reason ? ` (${props.parakeet!.reason})` : ""} —
          all languages will use Whisper.
        </p>
      </Show>
    </div>
  );
}

function LanguageCard(props: {
  title: string;
  subtitle: string;
  recommended: boolean;
  engine: "parakeet-local" | "whisper-local";
  selected: boolean;
  onSelect: () => void;
  children?: JSX.Element;
}) {
  const engineLabel = () =>
    props.engine === "parakeet-local" ? "Parakeet" : "Whisper";
  return (
    <div
      role="radio"
      aria-checked={props.selected}
      tabindex={0}
      onClick={props.onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          props.onSelect();
        }
      }}
      class="liquid-card flex cursor-pointer flex-col gap-1 rounded-[10px] bg-[var(--color-surface-raised)] px-3 py-2.5 text-left transition-[box-shadow,background-color] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-accent)]"
      classList={{
        "ring-1 ring-[var(--color-border)]": !props.selected,
        "ring-2 ring-[var(--color-accent)]": props.selected,
      }}
    >
      <div class="flex items-center justify-between gap-2">
        <span class="text-[13px] font-medium text-[var(--color-ink)]">
          {props.title}
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
            <span class="h-1.5 w-1.5 rounded-full bg-[var(--color-on-accent)]" />
          </Show>
        </span>
      </div>
      <span class="text-[11px] text-[var(--color-ink-mute)]">
        {props.subtitle}
      </span>
      <div class="mt-1 flex flex-wrap gap-1">
        <Show when={props.recommended}>
          <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
            Recommended
          </span>
        </Show>
        <span class="rounded-full bg-[var(--color-ink-faint)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-ink-mute)]">
          {engineLabel()}
        </span>
      </div>
      <Show when={props.children}>
        {/* Stop click bubbling so opening the dropdown doesn't re-fire
            the parent card's onSelect with the *previous* dropdown
            value. */}
        <div onClick={(e) => e.stopPropagation()}>{props.children}</div>
      </Show>
    </div>
  );
}

/// Per-id rolling history of (timestamp, bytes_downloaded) samples used
/// to derive a smoothed throughput estimate for the ETA. The window is
/// the most recent samples within `ETA_WINDOW_MS`; older entries are
/// dropped so the throughput reflects current speed, not a stale
/// resumed-download baseline.
type ProgressSample = { ts: number; bytes: number };
const ETA_WINDOW_MS = 5_000;
const ETA_MAX_SAMPLES = 12;

function DownloadStep(props: {
  s: Settings | null;
  tier: TierRecommendation | null;
  patch: (p: Partial<Settings>) => void;
  /// Parent owns the Continue gate; this step owns the IDs being downloaded.
  onInFlightChange: (v: boolean) => void;
  /// Tells the parent when every required artifact for the chosen
  /// engine is on disk. Continue stays disabled until this flips true.
  onReadyChange: (v: boolean) => void;
}) {
  const [progress, setProgress] = createSignal<
    Record<string, { downloaded: number; total: number }>
  >({});
  // Per-id rolling samples. Stored as a flat Record (not nested in
  // `progress`) so a ResizeObserver-style spread doesn't drop the
  // history on each tick.
  const [samples, setSamples] = createSignal<Record<string, ProgressSample[]>>(
    {},
  );
  const [completed, setCompleted] = createSignal<Set<string>>(new Set());
  const [started, setStarted] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);
  const [sttModels, setSttModels] = createSignal<ModelStatus[]>([]);
  const [llmModels, setLlmModels] = createSignal<ModelStatus[]>([]);
  const [selectedSttId, setSelectedSttId] = createSignal<string | null>(null);
  const [selectedLlmId, setSelectedLlmId] = createSignal<string | null>(null);
  const [binaryStatus, setBinaryStatus] =
    createSignal<InferenceServerStatus | null>(null);
  const [appleFm, setAppleFm] = createSignal<AppleFmAvailability | null>(null);
  // Always default to the local Gemma stack: Apple Intelligence is
  // faster but produces lower-quality polishing in our testing, so we
  // surface it as a tradeoff rather than the default.
  const [polishChoice, setPolishChoice] = createSignal<PolishChoice>(
    "gemma-local",
  );
  const unlistens: (() => void)[] = [];

  onMount(async () => {
    // model:* and binary:* emit the same payload shape, so handlers
    // share the progress/completed maps; per-id keys disambiguate.
    const handleProgress = (p: {
      id: string;
      bytes_downloaded: number;
      total_bytes: number;
    }) => {
      setProgress((prev) => ({
        ...prev,
        [p.id]: { downloaded: p.bytes_downloaded, total: p.total_bytes },
      }));
      const now = Date.now();
      setSamples((prev) => {
        const history = prev[p.id] ?? [];
        // Drop samples older than the smoothing window. The 200 ms
        // emit cadence from `models::stream_to_file` yields ~25
        // samples per 5 s, but a slow link can pace much slower —
        // keep the cap at ETA_MAX_SAMPLES regardless so we always
        // average over a few points instead of just one.
        const cutoff = now - ETA_WINDOW_MS;
        const trimmed = history
          .filter((s) => s.ts >= cutoff)
          .slice(-(ETA_MAX_SAMPLES - 1));
        trimmed.push({ ts: now, bytes: p.bytes_downloaded });
        return { ...prev, [p.id]: trimmed };
      });
    };
    const handleDone = (p: { id: string }) => {
      setCompleted((prev) => new Set(prev).add(p.id));
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
      const [all, bin, fm] = await Promise.all([
        listModels(),
        inferenceServerStatus(),
        appleFmAvailability(),
      ]);
      setSttModels(all.filter((m) => m.descriptor.kind === "stt"));
      setLlmModels(all.filter((m) => m.descriptor.kind === "llm"));
      setBinaryStatus(bin);
      setAppleFm(fm);
      // Re-runs via "Replay onboarding": pre-mark anything already
      // on disk so the requirements gate doesn't block forever and
      // returning users are not asked to re-download.
      setCompleted((prev) => {
        const next = new Set(prev);
        for (const m of all) {
          if (m.installed) next.add(m.descriptor.id);
        }
        if (bin.installed) next.add(bin.id);
        return next;
      });
    } catch (e) {
      console.warn("onboarding probes failed", e);
    }
  });

  /// Re-probe the filesystem and authoritatively update `completed`
  /// with everything actually on disk. Used as a safety net after the
  /// `Promise.all(downloads…)` resolves — the `model:done`/`binary:done`
  /// events flow over a separate IPC channel from the command response,
  /// and a slow listener can leave the readiness memo stale even after
  /// `download_model` returned Ok. Probing disk directly is the source
  /// of truth.
  async function reconcileCompletedFromDisk() {
    try {
      const [all, bin] = await Promise.all([
        listModels(),
        inferenceServerStatus(),
      ]);
      setSttModels(all.filter((m) => m.descriptor.kind === "stt"));
      setLlmModels(all.filter((m) => m.descriptor.kind === "llm"));
      setBinaryStatus(bin);
      setCompleted((prev) => {
        const next = new Set(prev);
        for (const m of all) {
          if (m.installed) next.add(m.descriptor.id);
        }
        if (bin.installed) next.add(bin.id);
        return next;
      });
    } catch (e) {
      console.warn("post-download disk reconcile failed", e);
    }
  }

  // Filter the STT options shown in the form to the engine the user
  // picked in the Language step — there's no point in offering Whisper
  // sizes when they're going to download Parakeet, and vice versa.
  // Falls back to "all STT models" before settings are loaded.
  const visibleSttModels = createMemo(() => {
    const all = sttModels();
    const eng = props.s?.stt_engine;
    if (!eng) return all;
    return all.filter((m) => m.descriptor.engine === eng);
  });

  // Auto-select default models. Prefer the user's persisted setting if
  // it still maps to a catalog entry (Replay onboarding), else the
  // hardware-tier recommendation. Runs once each — `selectedXId()` gate
  // makes the effect idempotent.
  createEffect(() => {
    const t = props.tier;
    const stts = visibleSttModels();
    const cur = props.s;
    if (!t || stts.length === 0 || selectedSttId()) return;
    const stored = cur?.stt_model_id;
    const ok = stored && stts.some((m) => m.descriptor.id === stored);
    setSelectedSttId(ok ? stored! : stts[0].descriptor.id);
  });

  createEffect(() => {
    const t = props.tier;
    const llms = llmModels();
    const cur = props.s;
    if (!t || llms.length === 0 || selectedLlmId()) return;
    const stored = cur?.llm_model_id;
    const ok = stored && llms.some((m) => m.descriptor.id === stored);
    setSelectedLlmId(ok ? stored! : t.llm_model_id);
  });

  // Compute "all required artifacts on disk" for the chosen engine and
  // forward it to the parent's Continue button. The user must click
  // the action button at least once (`started()`) so settings are
  // committed for their current selection — otherwise a returning
  // user could advance with stale `llm_engine` from a prior session.
  const requirementsMet = createMemo(() => {
    if (!started()) return false;
    const sttId = selectedSttId();
    const choice = polishChoice();
    const inst = completed();
    if (!sttId || !inst.has(sttId)) return false;
    if (choice === "apple-fm") return true;
    const llmId = selectedLlmId();
    const bin = binaryStatus();
    if (!llmId || !bin) return false;
    return inst.has(llmId) && inst.has(bin.id);
  });

  createEffect(() => {
    props.onReadyChange(requirementsMet());
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
    // Stop pretending the gate is met when this step unmounts —
    // otherwise a back-nav into the mic step would still advance.
    props.onReadyChange(false);
  });

  const selectedSttDescriptor = () => {
    const id = selectedSttId();
    if (!id) return null;
    return sttModels().find((m) => m.descriptor.id === id)?.descriptor ?? null;
  };
  const selectedLlmDescriptor = () => {
    const id = selectedLlmId();
    if (!id) return null;
    return llmModels().find((m) => m.descriptor.id === id)?.descriptor ?? null;
  };

  async function start() {
    const t = props.tier;
    const cur = props.s;
    const sttId = selectedSttId();
    const choice = polishChoice();
    if (!t || !cur || !sttId) return;
    setStarted(true);

    // Derive the engine id from the chosen model descriptor — the
    // user could have switched engines via the Language step but the
    // settings store hasn't echoed yet.
    const sttEngine =
      sttModels().find((m) => m.descriptor.id === sttId)?.descriptor.engine
        ?? cur.stt_engine
        ?? "whisper-local";

    if (choice === "apple-fm") {
      // Apple Intelligence: no GGUF download, no llama-server. STT
      // is still local and still needs to come down.
      await props.patch({
        stt_engine: sttEngine,
        stt_model_id: sttId,
        llm_engine: "apple-fm",
        // Keep the recommended Gemma id so a later switch in Settings
        // doesn't have to re-pick the tier-matched one. Honour the
        // user's prior pick if they had one; else fall back to tier.
        llm_model_id: selectedLlmId() ?? t.llm_model_id,
        polishing_enabled: true,
      });
      try {
        props.onInFlightChange(true);
        await downloadModel(sttId);
        // Reconcile against disk before pre-warm — the `model:done`
        // event arrives on a separate IPC channel from the command
        // response, and we don't want Continue to wait on a slow event
        // when the bytes are already on disk.
        await reconcileCompletedFromDisk();
        await loadModelWarm(sttId);
      } catch (e) {
        setErr(String(e));
      } finally {
        props.onInFlightChange(false);
      }
      return;
    }

    const llmId = selectedLlmId();
    if (!llmId) {
      setStarted(false);
      return;
    }
    if (!cur.gemma_tos_accepted) {
      await acceptGemmaTos();
    }
    await props.patch({
      stt_engine: sttEngine,
      stt_model_id: sttId,
      llm_engine: "gemma-local",
      llm_model_id: llmId,
      gemma_tos_accepted: true,
      polishing_enabled: true,
    });
    try {
      props.onInFlightChange(true);
      await Promise.all([
        downloadModel(sttId),
        downloadModel(llmId),
        downloadInferenceServer(),
      ]);
      // Reconcile against disk before pre-warm. Without this, a
      // `model:done` or `binary:done` event that arrives slowly leaves
      // `completed` stale, and the Continue gate stays disabled even
      // though every artifact is on disk — that was the original "must
      // click Back then Next" bug. Reading from `list_models` /
      // `inference_server_status` is the source of truth.
      await reconcileCompletedFromDisk();
      // Pre-warm so first dictation doesn't pay the cold-load tax. The
      // inference server is deferred until first polish — spawning it
      // now would burn RAM before the user even dictates.
      await Promise.allSettled([
        loadModelWarm(sttId),
        loadModelWarm(llmId),
      ]);
    } catch (e) {
      setErr(String(e));
    } finally {
      props.onInFlightChange(false);
    }
  }

  // Disable the action button until the user has made the necessary
  // selections. Once clicked, `started()` swaps the form for the
  // progress bars; the top-right Continue button takes over from there.
  const startDisabled = () =>
    !props.tier ||
    !selectedSttId() ||
    (polishChoice() === "gemma-local" && !selectedLlmId());

  // The wrapping <main> owns the scrollbar. Setting overflow-y here
  // would also activate horizontal clipping (browsers compute
  // overflow-x: auto when overflow-y is set with default x), which
  // pinches the selected card's `ring-2` accent at the box edge.
  return (
    <div class="flex w-full max-w-[440px] flex-col gap-4">
      <h2 class="text-[20px] font-semibold">Set up your dictation models.</h2>
      <Show when={!started()}>
        <p class="text-[13px] text-[var(--color-ink-mute)]">
          Vox runs everything on this Mac. Pick a transcription model and
          a polishing engine to clean each transcript up.
        </p>

        <Show when={visibleSttModels().length > 0 && props.tier}>
          {(t) => (
            <div
              class="flex flex-col gap-2"
              role="radiogroup"
              aria-label="Speech recognition model"
            >
              <div class="flex items-baseline justify-between">
                <span class="text-[13px] font-medium">
                  Speech recognition (
                  {props.s?.stt_engine === "parakeet-local"
                    ? "Parakeet"
                    : "Whisper"}
                  )
                </span>
                <Show when={t().physical_ram_gb > 0}>
                  <span class="text-[11px] text-[var(--color-ink-faint)]">
                    Your Mac has {t().physical_ram_gb} GB RAM
                  </span>
                </Show>
              </div>
              <For each={visibleSttModels()}>
                {(m) => (
                  <ModelCard
                    model={m}
                    selected={selectedSttId() === m.descriptor.id}
                    recommended={
                      m.descriptor.engine === "parakeet-local"
                        || m.descriptor.id === t().stt_model_id
                    }
                    swapWarn={
                      t().physical_ram_gb > 0 &&
                      m.descriptor.ram_peak_mb >
                        t().physical_ram_gb * 1024 * 0.85
                    }
                    onSelect={() => setSelectedSttId(m.descriptor.id)}
                  />
                )}
              </For>
            </div>
          )}
        </Show>

        <div
          class="flex flex-col gap-2"
          role="radiogroup"
          aria-label="Polishing engine"
        >
          <span class="text-[13px] font-medium">Transcript polishing</span>
          <PolishEngineCard
            engine="gemma-local"
            title="Gemma 4 (open-source)"
            subtitle="Runs entirely offline. Choose a size below."
            recommended={true}
            note={null}
            disabled={false}
            disabledReason={null}
            selected={polishChoice() === "gemma-local"}
            onSelect={() => setPolishChoice("gemma-local")}
          />
          <PolishEngineCard
            engine="apple-fm"
            title="Apple Intelligence"
            subtitle="On-device. No download. Requires macOS 26 + Apple Silicon."
            recommended={false}
            // Surfaces the quality tradeoff so the user picks the
            // tradeoff knowingly: Apple's Foundation Models polish
            // faster but produce lower-quality cleanups in our tests.
            note="Faster, but lower polishing quality."
            disabled={!(appleFm()?.available ?? false)}
            disabledReason={appleFm()?.message ?? null}
            selected={polishChoice() === "apple-fm"}
            onSelect={() => setPolishChoice("apple-fm")}
          />
        </div>

        <Show when={polishChoice() === "gemma-local" && llmModels().length > 0 && props.tier}>
          {(t) => (
            <div
              class="flex flex-col gap-2"
              role="radiogroup"
              aria-label="Gemma 4 model"
            >
              <span class="text-[13px] font-medium">Gemma 4 model</span>
              <For each={llmModels()}>
                {(m) => (
                  <ModelCard
                    model={m}
                    selected={selectedLlmId() === m.descriptor.id}
                    recommended={m.descriptor.id === t().llm_model_id}
                    swapWarn={
                      t().physical_ram_gb > 0 &&
                      m.descriptor.ram_peak_mb >
                        t().physical_ram_gb * 1024 * 0.85
                    }
                    onSelect={() => setSelectedLlmId(m.descriptor.id)}
                  />
                )}
              </For>
              <p class="text-[11px] text-[var(--color-ink-faint)]">
                By continuing you accept the{" "}
                <a
                  class="underline"
                  href="https://ai.google.dev/gemma/terms"
                  onClick={(e) => {
                    e.preventDefault();
                    void openExternalUrl("https://ai.google.dev/gemma/terms");
                  }}
                >
                  Gemma Terms of Use
                </a>
                .
              </p>
            </div>
          )}
        </Show>

        <button
          type="button"
          class="self-start rounded-[8px] bg-[var(--color-accent)] px-4 py-2 text-[13px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={startDisabled()}
          onClick={start}
        >
          {polishChoice() === "apple-fm"
            ? "Continue with Apple Intelligence"
            : "Accept and download"}
        </button>
      </Show>
      <Show when={started()}>
        <div class="space-y-3">
          <Show when={selectedSttDescriptor()}>
            {(d) => (
              <DownloadBar
                label={shortModelName(d().name)}
                done={completed().has(d().id)}
                prog={progress()[d().id]}
                samples={samples()[d().id]}
              />
            )}
          </Show>
          <Show when={polishChoice() === "gemma-local" && selectedLlmDescriptor()} fallback={null}>
            {(d) => (
              <DownloadBar
                label={shortModelName(d().name)}
                done={completed().has(d().id)}
                prog={progress()[d().id]}
                samples={samples()[d().id]}
              />
            )}
          </Show>
          <Show when={polishChoice() === "gemma-local" && binaryStatus()}>
            {(b) => (
              <DownloadBar
                label="Inference engine"
                done={completed().has(b().id)}
                prog={progress()[b().id]}
                samples={samples()[b().id]}
              />
            )}
          </Show>
        </div>
      </Show>
      <Show when={err()}>
        <p class="text-[12px] text-[var(--color-danger)]">{err()}</p>
      </Show>
    </div>
  );
}

function PolishEngineCard(props: {
  engine: PolishChoice;
  title: string;
  subtitle: string;
  recommended: boolean;
  /// Optional caveat pill rendered below the subtitle. Hidden when the
  /// card is disabled — the disabled-reason message takes precedence.
  note: string | null;
  disabled: boolean;
  disabledReason: string | null;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={props.selected}
      disabled={props.disabled}
      onClick={props.onSelect}
      class="liquid-card flex flex-col gap-1 rounded-[10px] bg-[var(--color-surface-raised)] px-3 py-2.5 text-left transition-[box-shadow,background-color] disabled:cursor-not-allowed"
      classList={{
        "ring-1 ring-[var(--color-border)]": !props.selected && !props.disabled,
        "ring-2 ring-[var(--color-accent)]": props.selected,
        "opacity-50": props.disabled,
      }}
    >
      <div class="flex items-center justify-between gap-2">
        <span class="text-[13px] font-medium text-[var(--color-ink)]">
          {props.title}
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
            <span class="h-1.5 w-1.5 rounded-full bg-[var(--color-on-accent)]" />
          </Show>
        </span>
      </div>
      <span class="text-[11px] text-[var(--color-ink-mute)]">
        {props.subtitle}
      </span>
      <Show
        when={
          (props.recommended && !props.disabled) ||
          (props.note && !props.disabled) ||
          (props.disabled && props.disabledReason)
        }
      >
        <div class="mt-1 flex flex-wrap gap-1">
          <Show when={props.recommended && !props.disabled}>
            <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
              Recommended
            </span>
          </Show>
          <Show when={props.note && !props.disabled}>
            <span class="inline-flex items-center gap-1 rounded-full bg-[var(--color-ink-faint)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-ink-mute)]">
              <svg
                width="10"
                height="10"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <circle cx="12" cy="12" r="10" />
                <line x1="12" y1="11" x2="12" y2="17" />
                <line x1="12" y1="7.5" x2="12" y2="7.51" />
              </svg>
              {props.note}
            </span>
          </Show>
          <Show when={props.disabled && props.disabledReason}>
            <span class="rounded-full bg-[var(--color-ink-faint)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-ink-mute)]">
              {props.disabledReason}
            </span>
          </Show>
        </div>
      </Show>
    </button>
  );
}

/// Generic model card used for both Whisper and Gemma rows. Sizing is
/// derived from `ModelDescriptor.size_bytes`, so MB-class Whisper
/// models render their disk footprint in MB rather than "0.1 GB".
function ModelCard(props: {
  model: ModelStatus;
  selected: boolean;
  recommended: boolean;
  swapWarn: boolean;
  onSelect: () => void;
}) {
  const d = () => props.model.descriptor;
  const displayName = () => d().name.replace(/\s*\([^)]*\)/g, "");
  const sizeLabel = () => {
    const bytes = d().size_bytes;
    if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB on disk`;
    return `${Math.round(bytes / 1e6)} MB on disk`;
  };
  const ramLabel = () => {
    const mb = d().ram_peak_mb;
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB peak RAM`;
    return `${mb} MB peak RAM`;
  };
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
            <span class="h-1.5 w-1.5 rounded-full bg-[var(--color-on-accent)]" />
          </Show>
        </span>
      </div>
      <span class="text-[11px] text-[var(--color-ink-mute)]">
        ~{sizeLabel()} · ~{ramLabel()}
      </span>
      <Show
        when={props.recommended || props.swapWarn || props.model.installed}
      >
        <div class="mt-1 flex flex-wrap gap-1">
          <Show when={props.recommended}>
            <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
              Recommended
            </span>
          </Show>
          <Show when={props.model.installed}>
            <span class="rounded-full bg-[var(--color-success)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-success)]">
              Installed
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

function shortModelName(name: string): string {
  // Keep the parenthesised quant tag (e.g. "(Q5_K_M)") so the
  // download bar matches the picker's full label. Only strip the
  // trailing em-dash tier suffix ("— Small"), which is duplicated by
  // the row's RAM/size hints elsewhere on the same screen.
  return name.replace(/ —.*$/, "");
}

/// Compute remaining seconds from a rolling sample window. Returns
/// `null` when there isn't enough signal yet — a single sample or a
/// flat span gives a nonsense throughput, and showing "0s remaining"
/// while a multi-GB GGUF still has 4 GB left to fetch would be worse
/// than no ETA. Throughput is the byte delta across the oldest and
/// newest sample in the window divided by their elapsed time, which is
/// effectively a 3–5 s moving average given the 200 ms emit cadence.
function etaSeconds(
  prog: { downloaded: number; total: number } | undefined,
  samples: ProgressSample[] | undefined,
): number | null {
  if (!prog || prog.total <= 0 || prog.downloaded >= prog.total) return null;
  if (!samples || samples.length < 2) return null;
  const oldest = samples[0];
  const newest = samples[samples.length - 1];
  const elapsedMs = newest.ts - oldest.ts;
  const deltaBytes = newest.bytes - oldest.bytes;
  if (elapsedMs <= 0 || deltaBytes <= 0) return null;
  const bps = (deltaBytes / elapsedMs) * 1000;
  const remaining = prog.total - prog.downloaded;
  const eta = remaining / bps;
  if (!isFinite(eta) || eta < 0) return null;
  return Math.round(eta);
}

/// Format seconds as `12s remaining`, `1m 23s remaining`, or
/// `1h 04m remaining`. Hours suppressed unless > 1h; minutes are
/// zero-padded only in the hours branch where two-digit alignment
/// keeps the eye from wobbling.
function formatEta(secs: number): string {
  if (secs < 60) return `${secs}s remaining`;
  if (secs < 3600) {
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    return `${m}m ${s.toString().padStart(2, "0")}s remaining`;
  }
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${m.toString().padStart(2, "0")}m remaining`;
}

function DownloadBar(props: {
  label: string;
  done: boolean;
  prog?: { downloaded: number; total: number };
  samples?: ProgressSample[];
}) {
  const pct = () => {
    if (props.done) return 100;
    if (!props.prog) return 0;
    if (props.prog.total <= 0) return 0;
    return Math.min(100, (props.prog.downloaded / props.prog.total) * 100);
  };
  const eta = () => {
    if (props.done) return null;
    return etaSeconds(props.prog, props.samples);
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
      <Show when={eta() !== null}>
        <div class="mt-1 text-[10px] text-[var(--color-ink-faint)]">
          {formatEta(eta()!)}
        </div>
      </Show>
    </div>
  );
}

function HowToUseStep(props: { s: Settings | null }) {
  // Render the BOUND hotkey (what the OS will fire), not the configured
  // one — they diverge when the configured key wasn't registerable and
  // the runtime fell back.
  const [diags, setDiags] = createSignal<Map<string, HotkeyDiagnostic>>(
    new Map(),
  );

  onMount(async () => {
    try {
      const d = await diagnoseHotkeys();
      setDiags(new Map(d.entries.map((e) => [e.purpose, e])));
    } catch (err) {
      console.warn("diagnoseHotkeys failed:", err);
    }
  });

  const ptt = () => diags().get("push-to-talk");
  const cycle = () => diags().get("mode-cycle");

  const pttGlyph = () =>
    formatHotkey(ptt()?.bound) ||
    formatHotkey(props.s?.hotkey) ||
    "⌘⌥.";
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
