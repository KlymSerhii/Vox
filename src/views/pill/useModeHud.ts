import { createSignal, onCleanup } from "solid-js";
import { MODE_CHIP_MS, MODE_HUD_MS } from "./constants";
import { type ActiveModePayload } from "./types";

export interface ModeHudDeps {
  // Tracks whether a recording session is currently in-flight. Used to
  // choose the companion chip (during a session) vs the full-takeover
  // HUD (idle mode-cycle).
  sessionActive: () => boolean;
}

/// Voice-mode HUD + chip presenter. The full-takeover HUD renders the
/// new mode in place of the dictation UI when the mode-cycle hotkey
/// fires while idle; the companion chip drops below the main pill when
/// it fires during an active recording session so the waveform stays
/// visible. Both are self-clearing on a timer.
export function useModeHud(deps: ModeHudDeps) {
  // Mode-cycle HUD: when set, the pill renders the new voice mode
  // (icon + name) instead of its usual dictation UI. Cleared after
  // MODE_HUD_MS or when a real session event arrives, whichever is
  // first, so a dictation that starts mid-HUD never has to share the
  // pill slot with a stale mode label.
  const [modeHud, setModeHud] = createSignal<ActiveModePayload | null>(null);
  let modeHudTimer: number | null = null;
  // Companion chip shown BELOW the main pill while a recording session
  // is active. Does NOT replace the waveform — both appear simultaneously.
  const [modeChip, setModeChip] = createSignal<ActiveModePayload | null>(null);
  const [modeChipFading, setModeChipFading] = createSignal(false);
  let modeChipTimer: number | null = null;
  let modeChipFadeTimer: number | null = null;

  function clearModeHud() {
    if (modeHudTimer !== null) {
      clearTimeout(modeHudTimer);
      modeHudTimer = null;
    }
    setModeHud(null);
  }

  function clearModeChip() {
    if (modeChipTimer !== null) {
      clearTimeout(modeChipTimer);
      modeChipTimer = null;
    }
    if (modeChipFadeTimer !== null) {
      clearTimeout(modeChipFadeTimer);
      modeChipFadeTimer = null;
    }
    setModeChipFading(false);
    setModeChip(null);
  }

  function showMode(payload: ActiveModePayload) {
    // During an active recording session show a companion chip below
    // the pill so the waveform stays visible. Outside a session use
    // the full-takeover HUD (shorter lifetime).
    if (deps.sessionActive()) {
      clearModeHud();
      // Clear any in-flight fade before showing a fresh chip.
      if (modeChipFadeTimer !== null) {
        clearTimeout(modeChipFadeTimer);
        modeChipFadeTimer = null;
      }
      setModeChipFading(false);
      setModeChip(payload);
      if (modeChipTimer !== null) clearTimeout(modeChipTimer);
      modeChipTimer = window.setTimeout(() => {
        modeChipTimer = null;
        // Fade out over 220 ms, then unmount.
        setModeChipFading(true);
        modeChipFadeTimer = window.setTimeout(() => {
          modeChipFadeTimer = null;
          setModeChip(null);
          setModeChipFading(false);
        }, 220);
      }, MODE_CHIP_MS);
    } else {
      clearModeChip();
      setModeHud(payload);
      if (modeHudTimer !== null) clearTimeout(modeHudTimer);
      modeHudTimer = window.setTimeout(() => {
        setModeHud(null);
        modeHudTimer = null;
      }, MODE_HUD_MS);
    }
  }

  onCleanup(() => {
    if (modeHudTimer !== null) {
      clearTimeout(modeHudTimer);
      modeHudTimer = null;
    }
    if (modeChipTimer !== null) {
      clearTimeout(modeChipTimer);
      modeChipTimer = null;
    }
    if (modeChipFadeTimer !== null) {
      clearTimeout(modeChipFadeTimer);
      modeChipFadeTimer = null;
    }
  });

  return {
    modeHud,
    modeChip,
    modeChipFading,
    clearModeHud,
    clearModeChip,
    showMode,
  };
}
