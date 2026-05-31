import { Show } from "solid-js";
import { isMac } from "@/lib/platform";
import { PermStatus } from "../components/PermStatus";

export function MicStep(props: { granted: boolean; onAllow: () => void }) {
  return (
    <div class="flex flex-col items-center gap-5 text-center">
      <h2 class="text-[20px] font-semibold">Let Vox hear you.</h2>
      <p class="max-w-[360px] text-[13px] text-[var(--color-ink-mute)]">
        Vox only captures audio while you pressed the hotkey. Nothing is saved
        to disk.
        {/* Windows has no TCC-style pre-grant prompt — cpal opens the
            device directly. If the global mic-privacy toggle is off, the
            first recording fails with a friendly pill instead. */}
        <Show when={!isMac}>
          {" "}If recording ever fails, enable microphone access in Windows
          Settings → Privacy → Microphone.
        </Show>
      </p>
      <button
        type="button"
        onClick={props.onAllow}
        class="rounded-[8px] bg-[var(--color-control)] px-4 py-2 text-[13px] hover:bg-[var(--color-control-hover)]"
      >
        {isMac ? "Allow microphone" : "Continue"}
      </button>
      <PermStatus
        granted={props.granted}
        grantedLabel={isMac ? "Granted" : "Ready"}
        pendingLabel="Waiting…"
      />
    </div>
  );
}
