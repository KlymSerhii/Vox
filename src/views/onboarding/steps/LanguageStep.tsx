import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import {
  languageDisplayName,
  PARAKEET_SUPPORTED_LANGUAGES,
  parakeetSupportsLanguage,
  recommendedSttEngineFor,
  type ParakeetAvailability,
  type Settings,
} from "@/lib/ipc";
import { LanguageCard } from "../components/LanguageCard";
import { deviceWord } from "@/lib/platform";
import { AUTO_VALUE, NON_PARAKEET_COMMON } from "../constants";

export function LanguageStep(props: {
  s: Settings | null;
  parakeet: ParakeetAvailability | null;
  patch: (p: Partial<Settings>) => Promise<void>;
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
          Parakeet isn't available on this {deviceWord}
          {props.parakeet!.reason ? ` (${props.parakeet!.reason})` : ""} —
          all languages will use Whisper.
        </p>
      </Show>
    </div>
  );
}
