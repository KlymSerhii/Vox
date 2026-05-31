import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  checkPermissions,
  openAccessibilitySettings,
  setAppearance,
  setAudioInputDevice,
  setAutoUpdateCheckEnabled,
  setLaunchAtLogin,
  type Settings,
} from "@/lib/ipc";
import { Card } from "../components/Card";
import { MicrophonePicker } from "../components/MicrophonePicker";
import { Row } from "../components/Row";
import { SectionHeader } from "../components/SectionHeader";
import { SegmentedControl } from "../components/SegmentedControl";
import { Toggle } from "@/components/Toggle";
import { isMac, isWindows } from "@/lib/platform";
import { LANGUAGE_OPTIONS } from "../constants";
import { friendlySttEngineName } from "../helpers";

export function GeneralTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  async function toggleLaunchAtLogin(v: boolean) {
    try {
      await setLaunchAtLogin(v);
      props.update({ launch_at_login: v });
    } catch (err) {
      console.error("Launch-at-login change failed:", err);
    }
  }

  async function changeAppearance(v: "light" | "dark") {
    try {
      await setAppearance(v);
      props.update({ appearance: v });
    } catch (err) {
      console.error("Appearance change failed:", err);
    }
  }

  // Auto-paste needs Accessibility — surface the live grant status
  // inline so the toggle UI stays honest about what's required.
  // `null` means we haven't probed yet; we treat that as "ungranted"
  // for nudging purposes until the first probe lands.
  const [accessibility, setAccessibility] = createSignal<boolean | null>(null);

  async function refreshAccessibility() {
    try {
      const r = await checkPermissions();
      setAccessibility(r.accessibility);
    } catch (e) {
      console.warn("checkPermissions failed:", e);
      setAccessibility(false);
    }
  }

  onMount(() => {
    void refreshAccessibility();
    // Re-probe when the user comes back to the Settings window after
    // (presumably) toggling Accessibility in System Settings. macOS
    // delivers a 'focus' event on the BrowserWindow when the app
    // regains key-window status.
    const onFocus = () => void refreshAccessibility();
    window.addEventListener("focus", onFocus);
    onCleanup(() => window.removeEventListener("focus", onFocus));
  });

  async function toggleAutoPaste(v: boolean) {
    props.update({ auto_paste_enabled: v });
    // Re-probe on flip-on so the nudge surfaces immediately if the
    // user enabled the toggle without granting Accessibility yet.
    if (v) {
      await refreshAccessibility();
    }
  }

  // Show the "needs Accessibility" sub-row when the toggle is on AND
  // we've confirmed the grant is missing. `accessibility() === null`
  // (first render, before the probe lands) is treated as "don't nudge
  // yet" to avoid a flash during normal use. Windows has no Accessibility
  // concept for SendInput — the Rust probe always reports granted there,
  // so this is already false; gating on `!isWindows` is belt-and-suspenders
  // so a future probe change can't surface a mac-only nudge on Windows.
  const showAccessibilityNudge = () =>
    !isWindows && props.s.auto_paste_enabled && accessibility() === false;

  return (
    <>
      <SectionHeader title="Recording" />
      <Card>
        <Row
          label="When you press the hotkey"
          hint="Hands-free toggles on a press; push-to-talk records while you hold."
          last
        >
          <SegmentedControl<"toggle" | "ptt">
            ariaLabel="Hotkey behavior"
            value={props.s.hands_free_enabled ? "toggle" : "ptt"}
            options={[
              { value: "toggle", label: "Hands-free" },
              { value: "ptt", label: "Push-to-talk" },
            ]}
            onChange={(v) =>
              props.update({ hands_free_enabled: v === "toggle" })
            }
          />
        </Row>
      </Card>

      <SectionHeader title="Voice" />
      <Card>
        <Row
          label="Spoken language"
          hint={`${friendlySttEngineName(props.s.stt_engine)} detects the language automatically when set to Auto.`}
        >
          <select
            class="rounded-[8px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border-strong)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s.language}
            onChange={(e) => props.update({ language: e.currentTarget.value })}
          >
            <For each={LANGUAGE_OPTIONS}>
              {(o) => <option value={o.value}>{o.label}</option>}
            </For>
          </select>
        </Row>
        <Row
          label="Suggest mode for the focused app"
          hint="Vox hints, never auto-switches."
        >
          <Toggle
            checked={props.s.auto_suggest_modes}
            onChange={(v) => props.update({ auto_suggest_modes: v })}
          />
        </Row>
        <Row
          label="Microphone"
          hint={
            isMac
              ? "Pick which input Vox records. Following the system default tracks your macOS Sound input setting."
              : "Pick which input Vox records. Following the system default tracks your Windows sound input setting."
          }
          last
        >
          <MicrophonePicker
            value={props.s.audio_input_device}
            onChange={(v) => {
              // Dedicated IPC (not `props.update` / set_settings) so
              // selecting a device also briefly opens it on the
              // backend — that's what fires the iPhone Continuity
              // Microphone activation banner. The settings:changed
              // emit reconciles `props.s` for us.
              void setAudioInputDevice(v);
            }}
          />
        </Row>
      </Card>

      <SectionHeader title="Appearance" />
      <Card>
        <Row label="Theme" hint="Switches all Vox windows immediately." last>
          <SegmentedControl<"light" | "dark">
            ariaLabel="Theme"
            value={props.s.appearance}
            options={[
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
            onChange={(v) => void changeAppearance(v)}
          />
        </Row>
      </Card>

      <SectionHeader title="Streaming pill" />
      <Card>
        <Row
          label="Show streaming pill"
          hint="See the transcript appear live in the pill after you release the hotkey."
        >
          <Toggle
            checked={props.s.streaming_pill_enabled}
            onChange={(v) => props.update({ streaming_pill_enabled: v })}
          />
        </Row>
        <Row
          label="Live transcript while speaking"
          hint="Replace the waveform with a live caption inside the pill. Uses more battery since it transcribes as you speak."
          last
        >
          <Toggle
            checked={props.s.live_transcript_enabled}
            onChange={(v) => props.update({ live_transcript_enabled: v })}
          />
        </Row>
      </Card>

      <SectionHeader title="Text insertion" />
      <Card>
        <Row
          label="Paste into focused field"
          hint={
            isMac
              ? "Vox sends ⌘V for you after polishing and restores your clipboard right after. Requires Accessibility."
              : "Vox sends Ctrl+V for you after polishing and restores your clipboard right after."
          }
          last={!showAccessibilityNudge()}
        >
          <Toggle
            checked={props.s.auto_paste_enabled}
            onChange={(v) => void toggleAutoPaste(v)}
          />
        </Row>
        <Show when={showAccessibilityNudge()}>
          <Row
            label="Accessibility not granted"
            hint="Open System Settings → Privacy & Security → Accessibility, then enable Vox. Vox falls back to clipboard-only until you do."
            last
          >
            <div class="flex shrink-0 items-center gap-2">
              <button
                type="button"
                class="rounded-[8px] bg-[var(--color-control)] px-3 py-1 text-[11px] font-medium text-[var(--color-ink)] hover:bg-[var(--color-control-hover)]"
                onClick={() => void openAccessibilitySettings()}
              >
                Open System Settings
              </button>
              <button
                type="button"
                class="rounded-[6px] px-2 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
                onClick={() => void refreshAccessibility()}
              >
                Re-check
              </button>
            </div>
          </Row>
        </Show>
      </Card>

      <SectionHeader title="Dock" />
      <Card>
        <Row
          label="Show Vox in the Dock"
          hint="Adds a Dock icon and a ⌘-Tab entry. Click it to open Settings. Vox always stays in the menu bar."
          last
        >
          <Toggle
            checked={props.s.show_dock_icon}
            onChange={(v) => props.update({ show_dock_icon: v })}
          />
        </Row>
      </Card>

      <SectionHeader title="Startup" />
      <Card>
        <Row
          label="Launch at login"
          hint="Vox starts quietly in your menu bar after sign-in."
        >
          <Toggle
            checked={props.s.launch_at_login}
            onChange={(v) => void toggleLaunchAtLogin(v)}
          />
        </Row>
        <Row
          label="Automatically check for updates"
          hint="Vox checks GitHub once a day in the background. Turn off to disable the periodic check; the manual button in About still works."
          last
        >
          <Toggle
            checked={props.s.auto_update_check_enabled}
            onChange={(v) => {
              props.update({ auto_update_check_enabled: v });
              void setAutoUpdateCheckEnabled(v).catch((err) =>
                console.error("setAutoUpdateCheckEnabled failed:", err),
              );
            }}
          />
        </Row>
      </Card>
    </>
  );
}
