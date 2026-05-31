import { VoxLogo } from "@/components/VoxLogo";
import { deviceWord } from "@/lib/platform";

export function Welcome() {
  return (
    <div class="flex flex-col items-center gap-5 text-center">
      <VoxLogo size={96} />
      <div>
        <h1 class="text-[24px] font-semibold">On-device dictation.</h1>
        <p class="mt-2 max-w-[360px] text-[13px] text-[var(--color-ink-mute)]">
          Press, speak, paste. Vox runs entirely on this {deviceWord} — nothing
          leaves the device.
        </p>
      </div>
    </div>
  );
}
