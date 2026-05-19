import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { listAudioInputDevices, type AudioInputDevice } from "@/lib/ipc";

/// Mic input picker for Settings → General → Microphone. Renders the
/// same `<select>` styling as the language dropdown above. The empty
/// value (`""`) means "follow the macOS system default" and is
/// persisted as `null`. A saved device that isn't currently
/// enumerated still renders ("not connected"), and the dictation
/// pipeline surfaces `AudioError::ConfiguredDeviceUnavailable` rather
/// than silently capturing from the default — so the user always
/// knows whether their explicit selection is in effect.
export function MicrophonePicker(props: {
  value: string | null;
  onChange: (v: string | null) => void;
}) {
  const [devices, setDevices] = createSignal<AudioInputDevice[]>([]);

  async function refresh() {
    try {
      setDevices(await listAudioInputDevices());
    } catch (e) {
      console.warn("listAudioInputDevices failed:", e);
    }
  }

  onMount(() => {
    void refresh();
    // Re-enumerate when the Settings window regains focus so plugging
    // in AirPods (or the user toggling Continuity on their iPhone)
    // doesn't leave the dropdown stale. Tray popover does the same.
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    onCleanup(() => window.removeEventListener("focus", onFocus));
  });

  const selected = () => props.value ?? "";
  const savedMissing = () => {
    const v = props.value;
    if (!v) return false;
    const list = devices();
    return list.length > 0 && !list.some((d) => d.name === v);
  };

  return (
    <select
      class="max-w-[260px] truncate rounded-[8px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border-strong)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
      value={selected()}
      onChange={(e) => {
        const v = e.currentTarget.value;
        props.onChange(v === "" ? null : v);
      }}
    >
      <option value="">Follow system default</option>
      <Show when={savedMissing()}>
        <option value={props.value ?? ""}>
          {props.value} (not connected)
        </option>
      </Show>
      <For each={devices()}>
        {(d) => (
          <option value={d.name}>
            {d.name}
            {d.is_default ? " · system default" : ""}
          </option>
        )}
      </For>
    </select>
  );
}
