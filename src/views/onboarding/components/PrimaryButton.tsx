import { Show } from "solid-js";
import { type PermissionStatus, type Settings } from "@/lib/ipc";
import { LAST_STEP, MIC_STEP, MODELS_STEP, type Step } from "../constants";

export function PrimaryButton(props: {
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
