import { RepolishShowcase } from "../components/RepolishShowcase";
import { VoiceModesShowcase } from "../components/VoiceModesShowcase";

export function TipsStep() {
  return (
    <div class="flex w-full max-w-[440px] flex-col gap-4 animate-pill-in">
      <header class="flex flex-col gap-1.5">
        <h2 class="text-[20px] font-semibold text-[var(--color-ink)]">
          Try a few tricks before you go.
        </h2>
        <p class="text-[13px] text-[var(--color-ink-mute)]">
          You can keep going without these — but they're what makes Vox feel personal.
        </p>
      </header>
      <VoiceModesShowcase />
      <RepolishShowcase />
      <p class="text-[11px] text-[var(--color-ink-faint)] mt-1">
        Edit, clone, or write your own modes in Settings → Modes.
      </p>
    </div>
  );
}
