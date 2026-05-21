import { createSignal, onCleanup } from "solid-js";
import {
  REVEAL_CHARS_PER_SEC,
  REVEAL_TICK_MS,
  TRANSCRIPT_HOLD_MS,
} from "./constants";
import { type PillState, type StreamPayload } from "./types";

export interface RevealDeps {
  reducedMotion: () => boolean;
  state: () => PillState;
  liveTranscriptEnabled: () => boolean;
  clearTranscriptHold: () => void;
  scheduleTranscriptHold: (durationMs: number) => void;
}

/// Rate-limited streaming reveal. Owns the visible/target text buffers
/// and the fixed-rate ticker that walks the visible slice toward the
/// cumulative buffer the backend has produced, so per-token jumps read
/// as a smooth flow instead of snapping. The slow-reveal book-keeping
/// implements the "smoothly reveal during the first two seconds" brief.
export function useReveal(deps: RevealDeps) {
  // Streaming UI state: only populated when the user has the streaming
  // pill enabled AND the active session is the pill's owner. The
  // backend filters server-side, but we double-check session_id to
  // ignore stragglers from a prior session.
  const [streamingText, setStreamingText] = createSignal<string>("");
  const [streamingMode, setStreamingMode] = createSignal<
    "transcript" | "polish" | null
  >(null);
  let currentSessionId: number | null = null;

  // Rate-limited reveal: targetText is the full cumulative buffer the
  // backend has produced so far; visibleText is the slice we're
  // currently showing. A fixed-rate ticker walks visibleText toward
  // targetText to keep visual flow smooth.
  let targetText = "";
  let revealTimer: number | null = null;
  let revealStartedAt = 0;
  let revealStartLen = 0;
  // Per-tick reveal rate. Defaults to REVEAL_CHARS_PER_SEC; on a final
  // transcript chunk with very little tail left to reveal, we drop the
  // rate so the post-stop animation still takes ≥ 2s instead of
  // snapping the last word in. Reset on every startReveal() call so
  // the slowdown only affects the stretch we explicitly slowed for.
  let revealRate = REVEAL_CHARS_PER_SEC;
  // Slow-reveal book-keeping. When the stage flips listening → transcribing
  // we kick a deliberately slow reveal sized so the *currently
  // accumulated* speculative transcript takes exactly TRANSCRIPT_HOLD_MS
  // to surface (the brief: "smoothly reveal during the first two
  // seconds"). After either the 2 s window elapses OR `targetText`
  // grows past the snapshot, the ticker rebases itself back to
  // REVEAL_CHARS_PER_SEC so any further incoming chars (whisper-final
  // tail, late polishing setup) don't inherit the slow rate.
  // `slowRevealActive` gates the rebase logic so it only fires once
  // per slow window — a non-slow `startReveal` clears the flag.
  let slowRevealActive = false;
  let slowRevealInitialTargetLen = 0;

  function stopReveal() {
    if (revealTimer !== null) {
      clearInterval(revealTimer);
      revealTimer = null;
    }
  }

  function flushReveal() {
    stopReveal();
    setStreamingText(targetText);
  }

  function startReveal(initialVisibleLen: number, rate?: number) {
    stopReveal();
    revealStartedAt = performance.now();
    revealStartLen = initialVisibleLen;
    revealRate = rate && rate > 0 ? rate : REVEAL_CHARS_PER_SEC;
    // A bare startReveal (no explicit rate) is the default-rate path —
    // make sure we're not still flagged as "in the slow 2s window"
    // from an earlier transcript-final slowdown.
    if (!rate || rate >= REVEAL_CHARS_PER_SEC) {
      slowRevealActive = false;
    }
    revealTimer = window.setInterval(() => {
      // Slow-reveal exit: once the 2 s window elapses OR the backend
      // has buffered more chars than we slowed for, rebase so the
      // remaining work runs at the default rate. Done by setting a
      // new origin (revealStartedAt / revealStartLen) and bumping
      // revealRate back up; the existing allowed-length math just
      // continues from there with no jump.
      if (slowRevealActive) {
        const elapsed = performance.now() - revealStartedAt;
        const grew = targetText.length > slowRevealInitialTargetLen;
        if (elapsed >= TRANSCRIPT_HOLD_MS || grew) {
          revealStartLen = streamingText().length;
          revealStartedAt = performance.now();
          revealRate = REVEAL_CHARS_PER_SEC;
          slowRevealActive = false;
        }
      }
      const elapsedSec = (performance.now() - revealStartedAt) / 1000;
      const allowedLen = Math.floor(revealStartLen + revealRate * elapsedSec);
      const cap = Math.min(allowedLen, targetText.length);
      const cur = streamingText();
      if (cap > cur.length) {
        setStreamingText(targetText.slice(0, cap));
      }
      if (cap >= targetText.length) {
        stopReveal();
      }
    }, REVEAL_TICK_MS);
  }

  function applyStreamPayload(
    payload: StreamPayload,
    mode: "transcript" | "polish",
  ) {
    if (deps.reducedMotion()) return;
    if (
      currentSessionId !== null &&
      payload.session_id !== currentSessionId
    ) {
      return;
    }
    currentSessionId = payload.session_id;

    // First polish chunk after a transcript stream — DON'T wipe.
    // Keeping the visible text in place lets the polish animate as a
    // left-to-right morph from the transcript instead of a
    // wipe-and-restart. The revealer continues from the current
    // visible length, walking toward the new polish targetText. For
    // typical polishes that share a prefix with the transcript (just
    // capitalization + punctuation tweaks), most characters literally
    // don't change.
    if (streamingMode() !== mode) {
      setStreamingMode(mode);
      revealStartLen = streamingText().length;
      // Filler-removal edge case: if the new polish target is shorter
      // than the visible transcript, snap visible to the polish's
      // length so the revealer doesn't try to "rewind".
      if (payload.text.length < streamingText().length) {
        setStreamingText(payload.text);
        revealStartLen = payload.text.length;
      }
      // Restart the reveal at the default rate. The slowed rate
      // computed for a transcript-final tail must NOT bleed across
      // into the polish stream — the polish's targetText is typically
      // much longer than the residual transcript tail, so inheriting
      // the slow rate would visibly lag the polish reveal. Resetting
      // `revealStartedAt` here also prevents the elapsed-time term
      // from compounding across the mode flip and snap-revealing the
      // first polish chunk.
      deps.clearTranscriptHold();
      stopReveal();
    }

    targetText = payload.text;

    // Same-mode shrink. A re-decoding engine (Parakeet) re-runs the
    // whole growing buffer each speculative pass and can return a
    // cumulative transcript SHORTER than what's already visible — it
    // re-segments and may drop a trailing word between passes. The
    // rate-limited revealer only ever EXTENDS visible text (it caps at
    // `targetText.length` but never rewinds), so without this it would
    // freeze on the stale longer text until a later pass grew past it.
    // Snap down to the shorter target and rebase the reveal origin so
    // the correction surfaces immediately. This is a no-op right after
    // a mode change (the block above already reconciled the length) and
    // during normal monotonic growth (target ≥ visible).
    if (targetText.length < streamingText().length) {
      setStreamingText(targetText);
      revealStartLen = targetText.length;
    }

    if (payload.final) {
      // Polish stream finals still snap immediately — the polishing UI
      // already paced the reveal, and a polish snap reads as "done"
      // intentionally. Only the transcript stream gets the smoothing
      // treatment, because the speculative decoder typically delivers
      // ~all of the text before the user releases the hotkey, leaving
      // a tiny tail that the legacy flush would pop in jarringly.
      if (mode !== "transcript") {
        deps.clearTranscriptHold();
        flushReveal();
        return;
      }

      const remainingChars = Math.max(
        0,
        targetText.length - streamingText().length,
      );
      if (remainingChars === 0) {
        // Visible text already caught up before the final chunk
        // arrived — nothing to animate. Still hold the transcript
        // view for ≥ TRANSCRIPT_HOLD_MS so the user has a beat to
        // read the result before the polishing UI overlays.
        stopReveal();
        deps.scheduleTranscriptHold(TRANSCRIPT_HOLD_MS);
        return;
      }
      const naturalCompletionSec = remainingChars / REVEAL_CHARS_PER_SEC;
      const targetCompletionSec = Math.max(
        TRANSCRIPT_HOLD_MS / 1000,
        naturalCompletionSec,
      );
      // Slow the rate ONLY when the natural rate would finish under
      // the minimum; otherwise leave it at the default. Either way,
      // schedule the hold so the polishing UI doesn't sneak in even
      // if the rate is unchanged (a long tail that finishes in 3s
      // shouldn't leave a 1s gap before polishing visibly takes
      // over — the hold matches the actual reveal duration).
      const slowedRate = remainingChars / targetCompletionSec;
      // The transcript-final tail manages its own pacing — clear the
      // listening→transcribing slow-reveal flag so the ticker doesn't
      // try to rebase out of *this* slowdown the moment elapsed
      // crosses TRANSCRIPT_HOLD_MS.
      slowRevealActive = false;
      startReveal(streamingText().length, slowedRate);
      deps.scheduleTranscriptHold(targetCompletionSec * 1000);
      return;
    }

    // Defer the visible reveal until the user releases the hotkey
    // UNLESS the user opted into the live-transcript view. With the
    // opt-in on, the speculative buffer surfaces at the default
    // reveal rate as it grows (no slow-walk — the user explicitly
    // asked for a live caption, not a delayed one). With the opt-in
    // off we keep the original behaviour: buffer silently and let
    // the listening → transcribing transition kick the slow 2 s
    // reveal so the brief's "smoothly reveal during the first two
    // seconds" promise holds.
    if (deps.state() === "listening" && !deps.liveTranscriptEnabled()) {
      return;
    }

    if (revealTimer === null) {
      startReveal(streamingText().length);
    }
  }

  // Listening → transcribing is the moment we promised the user the
  // speculative buffer would surface. If anything's piled up in
  // targetText (the typical case for utterances long enough that
  // whisper produced ≥ 1 chunk before hotkey release), kick a
  // deliberately slow reveal sized so the current snapshot lands in
  // TRANSCRIPT_HOLD_MS. The pacing floor is REVEAL_CHARS_PER_SEC so a
  // trivially short snapshot (3 chars) doesn't inch out at 1.5 cps.
  // The hold pins the polishing UI for the same window so the body
  // doesn't get overlaid mid-reveal. Caller invokes this only on the
  // listening → transcribing edge; the remaining guards live here.
  function kickSlowRevealForTranscribing() {
    if (deps.reducedMotion()) return;
    // Live-transcript users have already seen the speculative buffer
    // surface in real time during listening — kicking the deliberate
    // slow reveal here would re-throttle the next chars for ~2 s and
    // feel like a stutter. Let the default reveal continue.
    if (deps.liveTranscriptEnabled()) return;
    if (targetText.length === 0 || revealTimer !== null) return;
    // idealRate is "deliver every accumulated char inside the 2 s
    // hold window". For typical speculative buffers (a sentence or
    // two, ~50–300 chars) that lands BELOW REVEAL_CHARS_PER_SEC and
    // we genuinely want the slow walk. For trivially-short buffers
    // ("Hi.", 3 chars → 1.5 cps) it would crawl, so we FLOOR at
    // REVEAL_CHARS_PER_SEC so the snapshot snaps in at the normal
    // rate and the user sees the text immediately rather than
    // animating one letter every 670 ms. Long buffers (>360 chars)
    // exceed REVEAL_CHARS_PER_SEC naturally and reveal in <2 s — the
    // rebase logic ends the slow window the moment the snapshot is
    // fully on screen.
    const idealRate = targetText.length / (TRANSCRIPT_HOLD_MS / 1000);
    const slowedRate = Math.max(idealRate, REVEAL_CHARS_PER_SEC);
    slowRevealActive = true;
    slowRevealInitialTargetLen = targetText.length;
    startReveal(streamingText().length, slowedRate);
    // startReveal() clears slowRevealActive when the rate is at or
    // above default; restore the flag so the ticker still does its
    // time-based rebase even if the floor kicked in (we still want
    // the "after 2 s, switch back" semantics).
    slowRevealActive = true;
    deps.scheduleTranscriptHold(TRANSCRIPT_HOLD_MS);
  }

  function clearStreaming() {
    stopReveal();
    setStreamingText("");
    setStreamingMode(null);
    targetText = "";
    currentSessionId = null;
    slowRevealActive = false;
    slowRevealInitialTargetLen = 0;
    deps.clearTranscriptHold();
  }

  onCleanup(stopReveal);

  return {
    streamingText,
    streamingMode,
    applyStreamPayload,
    kickSlowRevealForTranscribing,
    clearStreaming,
  };
}
