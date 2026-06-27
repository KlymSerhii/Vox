import { createSignal, onCleanup } from "solid-js";

// Two-step confirm primitive for destructive actions. `window.confirm` is a
// silent no-op in Tauri's WKWebView, so the UI shows an "Are you sure?" state
// for `revertMs` after the first click and auto-disarms if a second click
// doesn't follow.
export function useConfirm(revertMs: number = 5000) {
  const [armed, setArmed] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function disarm() {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    setArmed(false);
  }

  function armOrFire(fire: () => void) {
    if (armed()) {
      disarm();
      fire();
    } else {
      setArmed(true);
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = undefined;
        setArmed(false);
      }, revertMs);
    }
  }

  onCleanup(() => {
    if (timer !== undefined) clearTimeout(timer);
  });

  return { armed, armOrFire, disarm };
}
