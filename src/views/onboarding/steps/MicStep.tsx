import { PermStatus } from "../components/PermStatus";

export function MicStep(props: { granted: boolean; onAllow: () => void }) {
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
