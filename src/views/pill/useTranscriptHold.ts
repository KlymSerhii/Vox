import { createSignal, onCleanup } from "solid-js";

/// Transcript hold: when set to a future performance.now() timestamp,
/// the pill keeps rendering as "transcribing" even if the backend
/// has already flipped the stage to "polishing". Used to enforce a
/// ≥ 2s minimum visible-transcript window after the user releases
/// the hotkey, so the existing speculative text doesn't pop and then
/// freeze into polishing instantly. A scheduled timer flips the
/// signal back to null at expiry to trigger the re-render.
export function useTranscriptHold() {
  const [transcriptHoldUntil, setTranscriptHoldUntil] = createSignal<
    number | null
  >(null);
  let transcriptHoldTimer: number | null = null;

  function clearTranscriptHold() {
    if (transcriptHoldTimer !== null) {
      clearTimeout(transcriptHoldTimer);
      transcriptHoldTimer = null;
    }
    setTranscriptHoldUntil(null);
  }

  function scheduleTranscriptHold(durationMs: number) {
    if (transcriptHoldTimer !== null) {
      clearTimeout(transcriptHoldTimer);
      transcriptHoldTimer = null;
    }
    const ms = Math.max(0, durationMs);
    setTranscriptHoldUntil(performance.now() + ms);
    transcriptHoldTimer = window.setTimeout(() => {
      transcriptHoldTimer = null;
      setTranscriptHoldUntil(null);
    }, ms);
  }

  // Active hold check: still active iff the deadline is in the future.
  // The scheduled timer flips the signal back to null on expiry, so
  // this becomes a cheap reactive read for downstream gating.
  const isTranscriptHoldActive = () => {
    const until = transcriptHoldUntil();
    if (until === null) return false;
    return performance.now() < until;
  };

  onCleanup(clearTranscriptHold);

  return {
    transcriptHoldUntil,
    isTranscriptHoldActive,
    clearTranscriptHold,
    scheduleTranscriptHold,
  };
}
