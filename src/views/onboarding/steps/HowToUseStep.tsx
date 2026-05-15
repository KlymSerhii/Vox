import { createSignal, onMount } from "solid-js";
import {
  diagnoseHotkeys,
  openAccessibilitySettings,
  type HotkeyDiagnostic,
  type Settings,
} from "@/lib/ipc";
import { HotkeyKeys } from "@/components/HotkeyKeys";
import { TipRow } from "../components/TipRow";
import { fellBackNote, pttFailed } from "../helpers";

export function HowToUseStep(props: { s: Settings | null }) {
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

  // Fall back chain: live-bound runtime value → user-configured spec
  // → hardcoded default. The hardcoded defaults match `Settings::default`.
  const pttSpec = () =>
    ptt()?.bound || props.s?.hotkey || "CommandOrControl+Alt+Period";
  const cycleSpec = () =>
    cycle()?.bound || props.s?.mode_cycle_hotkey || "Alt+M";

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
          label={<>{pttVerb()} <HotkeyKeys spec={pttSpec()} /></>}
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
          label={<HotkeyKeys spec={cycleSpec()} />}
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
