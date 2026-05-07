import {
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { pillResize } from "../lib/ipc";

type PillState =
  | "listening"
  | "transcribing"
  | "polishing"
  | "done"
  | "error"
  | "canceled"
  | "saturated";

interface PillStatePayload {
  state: PillState;
  message?: string;
  /** Soft-fail flag on `Done` — clipboard succeeded but a step
   *  (typically LLM polish) was substituted. Drives the amber
   *  warning treatment so the user notices their cleanup was
   *  skipped instead of mistaking it for a successful polish. */
  warning?: boolean;
  /** Count of background (non-foreground) in-flight sessions.
   *  Only present when non-zero — the field is omitted from the
   *  Rust payload via `skip_serializing_if`. */
  background_count?: number;
}

interface AudioLevelPayload {
  level: number;
}

interface ToastPayload {
  message: string;
  ms: number;
}

interface StreamPayload {
  session_id: number;
  text: string;
  final: boolean;
}

interface ActiveModePayload {
  id: string;
  name: string;
  icon: string;
}

/// Mode-cycle HUD lifetime for the full-takeover path (idle mode-cycle).
const MODE_HUD_MS = 800;
/// Companion chip lifetime when mode fires during an active recording
/// session — long enough to read comfortably while listening starts.
const MODE_CHIP_MS = 2000;

const BARS = 32;
/** Soft cap on how many characters per second the streaming UI
 * reveals. Tuned to slightly LAG Gemma's ~50 tok/s × ~4 char/tok =
 * ~200 cps generation rate so visible text doesn't snap per token
 * but doesn't fall multiple sentences behind either. 180 cps lets a
 * 200-char polish that streams in 1 s reveal in ~1.1 s — enough lead
 * for the eye to track without a jarring snap on `final: true`. */
const REVEAL_CHARS_PER_SEC = 180;
const REVEAL_TICK_MS = 33; // ~30 Hz

export default function PillView() {
  const [state, setState] = createSignal<PillState>("listening");
  const [message, setMessage] = createSignal<string | null>(null);
  const [warning, setWarning] = createSignal(false);
  const [levels, setLevels] = createSignal<number[]>(new Array(BARS).fill(0));
  const [backgroundCount, setBackgroundCount] = createSignal(0);
  const [toastMessage, setToastMessage] = createSignal<string | null>(null);
  // Tracks whether a recording session is currently in-flight.
  // Used by the mode-cycle handler to choose chip vs full-takeover HUD.
  // Starts false so a cold-start mode-cycle press correctly shows the HUD.
  const [sessionActive, setSessionActive] = createSignal(false);
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

  // Ref to the scrollable streaming body — used by the auto-scroll effect.
  let streamingBodyRef: HTMLDivElement | undefined;
  // True only when the body has actual overflow (old content has scrolled
  // out of view). Gates the top-fade mask so TypingDots / short text stay
  // fully visible until there is genuine content to hide.
  const [bodyOverflowing, setBodyOverflowing] = createSignal(false);

  // Streaming UI state: only populated when the user has the streaming
  // pill enabled AND the active session is the pill's owner. The
  // backend filters server-side, but we double-check session_id to
  // ignore stragglers from a prior session.
  const [streamingText, setStreamingText] = createSignal<string>("");
  const [streamingMode, setStreamingMode] = createSignal<
    "transcript" | "polish" | null
  >(null);
  let currentSessionId: number | null = null;
  // Reduced-motion users get the existing stage-label experience —
  // streaming animation is purely cosmetic and the rate-limited
  // reveal would feel sluggish without the smooth transition.
  // Tracked as a signal because macOS lets the user toggle "Reduce
  // motion" mid-session — a one-shot read at mount would persist a
  // stale answer for the rest of the process lifetime.
  const [reducedMotion, setReducedMotion] = createSignal(
    typeof window !== "undefined" &&
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  // Gates bar-height transitions during pill width animation. The pill
  // div runs a `transition-[width,box-shadow,padding,border-radius]
  // duration-200` on state change; the inner waveform bars run their
  // own per-bar height transitions at 110ms. Without coordination the
  // two run simultaneously and produce visible jitter as the bars
  // squeeze while the pill is still re-laying-out. While
  // `pillTransitioning()` is true, the `.pill-transitioning` class on
  // the pill root suppresses bar height transitions for the width
  // animation's 200ms window.
  const [pillTransitioning, setPillTransitioning] = createSignal(false);
  let pillTransitionTimer: number | null = null;
  let reducedMotionMql: MediaQueryList | null = null;
  let reducedMotionListener: ((e: MediaQueryListEvent) => void) | null = null;
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
  // Transcript hold: when set to a future performance.now() timestamp,
  // the pill keeps rendering as "transcribing" even if the backend
  // has already flipped the stage to "polishing". Used to enforce a
  // ≥ 2s minimum visible-transcript window after the user releases
  // the hotkey, so the existing speculative text doesn't pop and then
  // freeze into polishing instantly. A scheduled timer flips the
  // signal back to null at expiry to trigger the re-render.
  const [transcriptHoldUntil, setTranscriptHoldUntil] = createSignal<
    number | null
  >(null);
  let transcriptHoldTimer: number | null = null;

  /// Length of the post-final transcript hold. Picked so the user has
  /// a beat to read the final text before the polishing stage takes
  /// the slot — short enough to not feel like the app is hung when
  /// polish is fast.
  const TRANSCRIPT_HOLD_MS = 2000;

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
    if (reducedMotion()) return;
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
      clearTranscriptHold();
      stopReveal();
    }

    targetText = payload.text;

    if (payload.final) {
      // Polish stream finals still snap immediately — the polishing UI
      // already paced the reveal, and a polish snap reads as "done"
      // intentionally. Only the transcript stream gets the smoothing
      // treatment, because the speculative decoder typically delivers
      // ~all of the text before the user releases the hotkey, leaving
      // a tiny tail that the legacy flush would pop in jarringly.
      if (mode !== "transcript") {
        clearTranscriptHold();
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
        scheduleTranscriptHold(TRANSCRIPT_HOLD_MS);
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
      scheduleTranscriptHold(targetCompletionSec * 1000);
      return;
    }

    // Defer the visible reveal until the user releases the hotkey.
    // Speculative Whisper output keeps streaming during `listening` for
    // the CPU win, but we don't show it — the brief: "I want that
    // initial transcription to be revealed smoothly during the first
    // two seconds" *after* the user finishes speaking. When the
    // pill:state listener fires `listening → transcribing` it will
    // kick the deliberate slow reveal that consumes whatever's piled
    // up in `targetText`.
    if (state() === "listening") {
      return;
    }

    if (revealTimer === null) {
      startReveal(streamingText().length);
    }
  }

  function clearStreaming() {
    stopReveal();
    setStreamingText("");
    setStreamingMode(null);
    targetText = "";
    currentSessionId = null;
    slowRevealActive = false;
    slowRevealInitialTargetLen = 0;
    clearTranscriptHold();
  }

  let unlisten: UnlistenFn[] = [];
  let toastTimer: number | null = null;

  onMount(async () => {
    document.body.classList.add("translucent");

    // Live `prefers-reduced-motion`: macOS users can toggle "Reduce
    // motion" in System Settings → Accessibility → Display while Vox
    // is running. matchMedia.addEventListener gives us a "change"
    // notification so the streaming pill respects the new preference
    // on the very next session — no Vox restart needed.
    if (
      typeof window !== "undefined" &&
      typeof window.matchMedia === "function"
    ) {
      reducedMotionMql = window.matchMedia(
        "(prefers-reduced-motion: reduce)",
      );
      reducedMotionListener = (e: MediaQueryListEvent) => {
        setReducedMotion(e.matches);
        // If the user just opted INTO reduced motion mid-stream, drop
        // any in-flight reveal so the pill collapses back to the
        // stage-label treatment cleanly.
        if (e.matches) {
          clearStreaming();
        }
      };
      reducedMotionMql.addEventListener("change", reducedMotionListener);
    }

    unlisten.push(
      await listen<PillStatePayload>("pill:state", (e) => {
        // Real session activity wins back the pill slot from any active HUD.
        clearModeHud();
        // Chip is also cleared on terminal states so it doesn't linger
        // into Done/Error/Canceled.
        if (
          e.payload.state === "done" ||
          e.payload.state === "error" ||
          e.payload.state === "canceled"
        ) {
          clearModeChip();
          setSessionActive(false);
        }
        if (e.payload.state === "listening") {
          setSessionActive(true);
        }
        const prevState = state();
        setState(e.payload.state);
        setMessage(e.payload.message ?? null);
        setWarning(e.payload.warning ?? false);
        setBackgroundCount(e.payload.background_count ?? 0);
        // Width transition window — disable inner bar height
        // transitions for the 200ms while the pill resizes so they
        // don't jitter alongside the layout flip.
        if (prevState !== e.payload.state) {
          setPillTransitioning(true);
          if (pillTransitionTimer !== null) {
            clearTimeout(pillTransitionTimer);
          }
          pillTransitionTimer = window.setTimeout(() => {
            setPillTransitioning(false);
            pillTransitionTimer = null;
          }, 220);
        }
        // Listening starts a brand-new utterance — drop any leftover
        // streaming buffers from a stale session that finalised.
        if (e.payload.state === "listening") {
          clearStreaming();
        }
        // Listening → transcribing is the moment we promised the user
        // the speculative buffer would surface. If anything's piled
        // up in targetText (the typical case for utterances long
        // enough that whisper produced ≥ 1 chunk before hotkey
        // release), kick a deliberately slow reveal sized so the
        // current snapshot lands in TRANSCRIPT_HOLD_MS. The pacing
        // floor is REVEAL_CHARS_PER_SEC so a trivially short snapshot
        // (3 chars) doesn't inch out at 1.5 cps. The hold pins the
        // polishing UI for the same window so the body doesn't get
        // overlaid mid-reveal.
        if (
          prevState === "listening" &&
          e.payload.state === "transcribing" &&
          !reducedMotion() &&
          targetText.length > 0 &&
          revealTimer === null
        ) {
          // idealRate is "deliver every accumulated char inside the 2 s
          // hold window". For typical speculative buffers (a sentence
          // or two, ~50–300 chars) that lands BELOW REVEAL_CHARS_PER_SEC
          // and we genuinely want the slow walk. For trivially-short
          // buffers ("Hi.", 3 chars → 1.5 cps) it would crawl, so we
          // FLOOR at REVEAL_CHARS_PER_SEC so the snapshot snaps in at
          // the normal rate (~16 ms here) and the user sees the text
          // immediately rather than animating one letter every 670 ms.
          // Long buffers (>360 chars) exceed REVEAL_CHARS_PER_SEC
          // naturally and reveal in <2 s — the rebase logic below ends
          // the slow window the moment the snapshot is fully on screen.
          const idealRate =
            targetText.length / (TRANSCRIPT_HOLD_MS / 1000);
          const slowedRate = Math.max(idealRate, REVEAL_CHARS_PER_SEC);
          slowRevealActive = true;
          slowRevealInitialTargetLen = targetText.length;
          startReveal(streamingText().length, slowedRate);
          // startReveal() clears slowRevealActive when the rate is at
          // or above default; restore the flag so the ticker still
          // does its time-based rebase even if the floor kicked in
          // (we still want the "after 2 s, switch back" semantics).
          slowRevealActive = true;
          scheduleTranscriptHold(TRANSCRIPT_HOLD_MS);
        }
        if (
          e.payload.state === "done" ||
          e.payload.state === "error" ||
          e.payload.state === "canceled"
        ) {
          // Fade the streaming text out at terminal — the success /
          // warning treatment + label takes over the slot.
          clearStreaming();
        }
      }),
    );
    unlisten.push(
      await listen<AudioLevelPayload>("pill:audio_level", (e) => {
        const v = clamp(e.payload.level);
        setLevels((prev) => [...prev.slice(1), v]);
      }),
    );
    unlisten.push(
      await listen("pill:show", () => {
        setLevels(new Array(BARS).fill(0));
        setWarning(false);
        clearStreaming();
      }),
    );
    unlisten.push(
      await listen<ToastPayload>("pill:toast", (e) => {
        setState("saturated");
        setToastMessage(e.payload.message);
        if (toastTimer !== null) {
          clearTimeout(toastTimer);
        }
        toastTimer = window.setTimeout(() => {
          setToastMessage(null);
          toastTimer = null;
        }, e.payload.ms);
      }),
    );
    unlisten.push(
      await listen<StreamPayload>("pill:transcript_stream", (e) => {
        applyStreamPayload(e.payload, "transcript");
      }),
    );
    unlisten.push(
      await listen<StreamPayload>("pill:polish_stream", (e) => {
        applyStreamPayload(e.payload, "polish");
      }),
    );
    unlisten.push(
      await listen<ActiveModePayload>("pill:active_mode", (e) => {
        // During an active recording session show a companion chip
        // below the pill so the waveform stays visible. Outside a
        // session use the full-takeover HUD (shorter lifetime).
        if (sessionActive()) {
          clearModeHud();
          // Clear any in-flight fade before showing a fresh chip.
          if (modeChipFadeTimer !== null) {
            clearTimeout(modeChipFadeTimer);
            modeChipFadeTimer = null;
          }
          setModeChipFading(false);
          setModeChip(e.payload);
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
          setModeHud(e.payload);
          if (modeHudTimer !== null) clearTimeout(modeHudTimer);
          modeHudTimer = window.setTimeout(() => {
            setModeHud(null);
            modeHudTimer = null;
          }, MODE_HUD_MS);
        }
      }),
    );
  });

  onCleanup(() => {
    for (const u of unlisten) u();
    setSessionActive(false);
    if (toastTimer !== null) {
      clearTimeout(toastTimer);
    }
    if (pillTransitionTimer !== null) {
      clearTimeout(pillTransitionTimer);
      pillTransitionTimer = null;
    }
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
    stopReveal();
    clearTranscriptHold();
    teardownResizeObserver();
    if (reducedMotionMql && reducedMotionListener) {
      reducedMotionMql.removeEventListener("change", reducedMotionListener);
      reducedMotionMql = null;
      reducedMotionListener = null;
    }
  });

  const isDone = () => state() === "done";
  const isDoneWarning = () => isDone() && warning();
  const isDoneSuccess = () => isDone() && !warning();
  // Active hold check: still active iff the deadline is in the future.
  // The scheduled timer flips the signal back to null on expiry, so
  // this becomes a cheap reactive read for downstream gating.
  const isTranscriptHoldActive = () => {
    const until = transcriptHoldUntil();
    if (until === null) return false;
    return performance.now() < until;
  };
  // `effectiveState()` is `state()` everywhere EXCEPT during the
  // post-final transcript hold: while the hold is active we keep
  // reporting "transcribing" even if the backend already advanced
  // to "polishing", so the pill UI doesn't pop into the polishing
  // treatment before the user has had ≥ TRANSCRIPT_HOLD_MS to read
  // the final transcript text. The polish stream itself can still
  // arrive in the background (it'll be applied to targetText), but
  // every UI predicate that gates on "are we polishing?" funnels
  // through this memo so the visual transition is delayed in lockstep.
  const effectiveState = createMemo<PillState>(() => {
    if (state() === "polishing" && isTranscriptHoldActive()) {
      return "transcribing";
    }
    return state();
  });
  const isProcessing = () =>
    effectiveState() === "transcribing" ||
    effectiveState() === "polishing";
  const isSaturated = () => state() === "saturated";
  const isStreaming = () =>
    isProcessing() && streamingMode() !== null && streamingText().length > 0;
  // True when the transcript stream finalised, the backend flipped the
  // stage to Polishing, but the LLM hasn't emitted its first chunk yet
  // (TTFT — model is processing the prompt). Without this signal the
  // pill goes static for 1–3 s and reads as "frozen". We use it to
  // shimmer the visible transcript text in the polish tint so the user
  // sees the model is actively chewing on what they just said. Gated
  // on `effectiveState()` so the transcript hold suppresses the
  // shimmer for the same window it suppresses the polishing label.
  const isWaitingForPolish = () =>
    effectiveState() === "polishing" &&
    streamingMode() === "transcript" &&
    streamingText().length > 0;

  // Listening swaps the meter to warm coral so the brand color earns
  // its keep at the one moment the device is recording. DESIGN.md §5.4.
  // Keys on `effectiveState()` so the post-final transcript hold keeps
  // the cyan tint instead of popping to violet the instant the
  // backend flips the stage.
  const meterColor = createMemo(() => {
    switch (effectiveState()) {
      case "listening":
        return "var(--color-coral)";
      case "transcribing":
        // Cyan matches the streaming caret tint for the same pipeline
        // stage — previously this returned --color-accent (teal),
        // which drifted from the cyan caret + glow used in
        // StreamingText. Same stage, same colour. The Waveform path
        // for the listening state continues to receive coral, so this
        // change is scoped to the transcribing label colour only.
        return "var(--color-cyan)";
      case "polishing":
        return "var(--color-violet)";
      default:
        return "var(--color-meter)";
    }
  });

  // Bridge from the rendered DOM up to the OS window: when the
  // streaming text grows wider or wraps to N lines, the inner pill
  // div's measured size grows; we push that to Rust via `pill_resize`
  // so the window chrome matches. Width drives how snug the pill hugs
  // short polishes; height handles multi-line wrapping. Without this,
  // either dimension would clip past the OS window's compact frame.
  //
  // wrapperRef points to the outer flex-column container that holds
  // both the pill body and the optional mode chip below it. The
  // ResizeObserver watches the wrapper so pill_resize accounts for the
  // chip's extra height; pillRef still targets the inner pill div for
  // all the imperative min-width style writes.
  let wrapperRef: HTMLDivElement | undefined;
  let pillRef: HTMLDivElement | undefined;
  let resizeObserver: ResizeObserver | null = null;
  let resizeRaf: number | null = null;
  // null sentinel = "no size reported yet" so the very first observer
  // emit always wins the change check. Replaces the previous magic
  // 144/36 placeholder, which leaked the compact-pill dimensions into
  // the dedup logic.
  let lastReportedWidth: number | null = null;
  let lastReportedHeight: number | null = null;
  // Window outer frame is 16 px wider and 10 px taller than the
  // visible pill so the OS drop shadow + 1 px rim have room to fall
  // off. Mirror that here when reporting back so Rust resizes to the
  // right outer dimensions.
  const WINDOW_PADDING_W = 16;
  const WINDOW_PADDING_H = 10;
  const STREAM_GROW_ENABLED = () => isStreaming() || isWaitingForPolish();

  // ─── Dynamic width-floor lerp ──────────────────────────────────────
  // Couples width to height so a tall, narrow pill never happens. As
  // the streaming content grows in height, the pill's *minimum* width
  // ramps up via an ease-out curve, forcing the box to widen rather
  // than balloon vertically. `fit-content` still allows the natural
  // width to exceed the floor when content needs more.
  //
  // H_min/H_max bracket the ramp; H_max should track HEIGHT_MAX in
  // pill.rs (120). W_min is 180 px — single-line streaming feels
  // intentional at that width. W_max is 360 px.
  const H_MIN = 32;
  const H_MAX = 120;
  const W_MIN = 180;
  const W_MAX = 360;
  // 12 px quantisation grid + 12 px grow / 32 px shrink hysteresis
  // bands stop the floor from chasing every sub-pixel content nudge
  // (each one would otherwise round-trip to Rust as a window resize).
  const FLOOR_GRID = 12;
  const FLOOR_GROW_TRIGGER = 12;
  const FLOOR_SHRINK_TRIGGER = 32;
  // Sustain time before a shrink commits. Measured in ms via
  // performance.now() at the rAF callback; a sub-32 px content
  // height drop has to persist this long before the floor steps down.
  const FLOOR_SHRINK_SUSTAIN_MS = 250;
  // Signal so the floor lives inside the same reactive style object
  // SolidJS rebuilds on every state change — writing to `pillRef.style.minWidth`
  // imperatively would be clobbered the next time any other style key
  // (e.g. box-shadow flipping on Done) flagged the parent for re-render.
  // 0 is the sentinel "not yet applied" — the parent style attribute's
  // own `min-width` rule wins until the observer raises this above 0.
  const [dynamicMinWidth, setDynamicMinWidth] = createSignal(0);
  let lastAppliedFloor = 0;
  let pendingShrinkFloor: number | null = null;
  let pendingShrinkSince = 0;

  function computeWidthFloor(measuredH: number): number {
    const rawT = (measuredH - H_MIN) / (H_MAX - H_MIN);
    const t = Math.max(0, Math.min(1, rawT));
    // Ease-out: 1 - (1 - t)^2 — fast initial widening so even slight
    // overflow past H_MIN (≈single line) starts opening the box.
    const tEased = 1 - (1 - t) * (1 - t);
    const floor = W_MIN + (W_MAX - W_MIN) * tEased;
    return Math.round(floor / FLOOR_GRID) * FLOOR_GRID;
  }

  function applyDynamicWidthFloor(measuredH: number) {
    if (!pillRef) return;
    const candidate = computeWidthFloor(measuredH);
    const delta = candidate - lastAppliedFloor;
    if (delta === 0) return;
    if (delta >= FLOOR_GROW_TRIGGER) {
      // Grow path commits immediately — content already overflowed,
      // any wait would visibly lag the wrap.
      pendingShrinkFloor = null;
      lastAppliedFloor = candidate;
      setDynamicMinWidth(candidate);
      return;
    }
    if (delta > 0) {
      // Within the dead-band — ignore so we don't churn IPC.
      pendingShrinkFloor = null;
      return;
    }
    // delta < 0: candidate floor is smaller than what we've applied.
    // Only commit shrinks that exceed the wider band AND have been
    // sustained for FLOOR_SHRINK_SUSTAIN_MS — protects against a
    // momentary content height dip yanking the floor down only to
    // immediately bounce back.
    if (-delta < FLOOR_SHRINK_TRIGGER) {
      pendingShrinkFloor = null;
      return;
    }
    const now = performance.now();
    if (pendingShrinkFloor !== candidate) {
      pendingShrinkFloor = candidate;
      pendingShrinkSince = now;
      return;
    }
    if (now - pendingShrinkSince < FLOOR_SHRINK_SUSTAIN_MS) {
      return;
    }
    lastAppliedFloor = candidate;
    pendingShrinkFloor = null;
    setDynamicMinWidth(candidate);
  }

  function resetDynamicWidthFloor() {
    lastAppliedFloor = 0;
    pendingShrinkFloor = null;
    pendingShrinkSince = 0;
    setDynamicMinWidth(0);
  }

  function reportPillSize(contentW: number, contentH: number) {
    // Lower bound matches the Rust WIDE_WIDTH_MIN_DYNAMIC (W_MIN px).
    // Anything below would be clamped up by Rust anyway and just
    // produce an extra IPC round-trip.
    const targetW = Math.max(W_MIN, Math.ceil(contentW + WINDOW_PADDING_W));
    const targetH = Math.max(36, Math.ceil(contentH + WINDOW_PADDING_H));
    // Round to 4 px to soak up sub-pixel jitter from the browser's
    // layout pass — without this the IPC fires on every keystroke.
    const roundedW = Math.ceil(targetW / 4) * 4;
    const roundedH = Math.ceil(targetH / 4) * 4;
    const widthChanged = roundedW !== lastReportedWidth;
    const heightChanged = roundedH !== lastReportedHeight;
    if (!widthChanged && !heightChanged) return;
    lastReportedWidth = roundedW;
    lastReportedHeight = roundedH;
    void pillResize({
      width: widthChanged ? roundedW : undefined,
      height: heightChanged ? roundedH : undefined,
    }).catch(() => {
      // Ignore — IPC failure here just means the window stays the
      // size it was. Worst case the text overflows.
    });
  }

  function setupResizeObserver() {
    if (resizeObserver || !wrapperRef) return;
    resizeObserver = new ResizeObserver((entries) => {
      // Coalesce bursts via rAF so a flurry of streaming chunks only
      // produces one pill_resize per frame.
      if (resizeRaf !== null) cancelAnimationFrame(resizeRaf);
      resizeRaf = requestAnimationFrame(() => {
        resizeRaf = null;
        const last = entries[entries.length - 1];
        if (!last) return;
        // Update the dynamic width floor BEFORE reporting size up so
        // the very next layout pass — which the floor change kicks
        // off — gets reported instead of the pre-floor measurement.
        // Use the inner pill height (not wrapper height) so the chip's
        // extra ~29 px (gap + chip row) doesn't skew the floor lerp.
        const pillHeight = pillRef
          ? pillRef.getBoundingClientRect().height
          : last.contentRect.height;
        applyDynamicWidthFloor(pillHeight);
        // reportPillSize still gets the wrapper's full height so the
        // OS window accounts for the chip when it's visible.
        reportPillSize(last.contentRect.width, last.contentRect.height);
      });
    });
    resizeObserver.observe(wrapperRef);
  }

  function teardownResizeObserver() {
    if (resizeObserver) {
      resizeObserver.disconnect();
      resizeObserver = null;
    }
    if (resizeRaf !== null) {
      cancelAnimationFrame(resizeRaf);
      resizeRaf = null;
    }
    // Reset cached dimensions to the null sentinel so the next
    // streaming session reports fresh measurements rather than
    // skipping due to "no change".
    lastReportedWidth = null;
    lastReportedHeight = null;
    resetDynamicWidthFloor();
  }

  // Auto-scroll the streaming body to bottom when new text arrives so
  // the newest tokens are always visible. Also tracks whether the body
  // is genuinely overflowing so the top-fade mask only applies then.
  createEffect(() => {
    const text = streamingText(); // reactive dependency
    if (!streamingBodyRef) return;
    // Reset overflow flag when there is no text (new session boundary).
    if (!text) {
      setBodyOverflowing(false);
      return;
    }
    requestAnimationFrame(() => {
      if (!streamingBodyRef) return;
      streamingBodyRef.scrollTop = streamingBodyRef.scrollHeight;
      setBodyOverflowing(streamingBodyRef.scrollHeight > streamingBodyRef.clientHeight + 1);
    });
  });

  // Toggle the observer with the streaming lifecycle so we don't push
  // height updates for the compact Listening/Done states (Rust handles
  // those via set_wide).
  let wasStreaming = false;
  createEffect(() => {
    const streamingNow = STREAM_GROW_ENABLED();
    if (streamingNow) {
      wasStreaming = true;
      setupResizeObserver();
    } else {
      teardownResizeObserver();
      if (wasStreaming) {
        wasStreaming = false;
        // Pre-empt the Rust-side `set_wide(false)` IPC: terminal
        // states (Done/Error/Canceled) still render the wide pill for
        // ~150 ms after streaming ends because the state event and
        // the resize event arrive at the webview through different
        // async paths. A compact-dim pillResize request is intercepted
        // by Rust's `set_size` (below-floor escape hatch) and routed
        // straight to the compact silhouette so the TerminalLine
        // renders at 144x36 instead of centred in a 600 px box.
        const COMPACT_W = 116 + WINDOW_PADDING_W;
        const COMPACT_H = 36 + WINDOW_PADDING_H;
        void pillResize({
          width: COMPACT_W,
          height: COMPACT_H,
        }).catch(() => {});
      }
    }
  });

  // Option (b): drive explicit height IPC from modeChip/modeChipFading signals.
  // The ResizeObserver is intentionally off during listening (compact pill, 116px
  // wide — always-on observer would report W_MIN=180 and force a wide-mode resize).
  // Instead we watch the chip signals directly and push compact-height updates.
  createEffect(() => {
    const chip = modeChip();
    const fading = modeChipFading();
    // Only active when streaming is off (listening phase, compact pill).
    if (STREAM_GROW_ENABLED()) return;
    if (state() !== "listening") return;
    if (chip) {
      // Pill (36px) + drawer (25px) + WINDOW_PADDING_H (10px) = 71 → round to 72.
      void pillResize({ height: 72 }).catch(() => {});
    } else if (!fading) {
      // Drawer gone — restore compact height.
      void pillResize({ height: 36 + WINDOW_PADDING_H }).catch(() => {});
    }
  });

  return (
    <div class="flex h-screen w-screen items-center justify-center bg-transparent">
      <div ref={(el) => (wrapperRef = el)} class="flex flex-col items-center">
      <div
        ref={(el) => (pillRef = el)}
        class="animate-pill-in pointer-events-none relative flex min-h-[26px] items-center rounded-[14px] px-3 py-1.5 transition-[width,box-shadow,padding,border-radius] duration-200 ease-out"
        classList={{ "pill-transitioning": pillTransitioning() }}
        style={{
          // Column layout during any processing stage so the sticky
          // header sits above the scrollable streaming body. Other
          // states stay row-centred.
          "flex-direction":
            isProcessing() ? "column" : "row",
          "justify-content":
            isProcessing() ? "flex-start" : "center",
          "align-items":
            isProcessing() ? "stretch" : "center",
          // Compact pill sits at 116 px. While streaming we use
          // `fit-content` so the pill div sizes itself to the actual
          // text width; min/max clamp the box at a 180 px floor
          // (single-line streaming feels intentional) and a 360 px
          // ceiling (matches WIDE_WIDTH_MAX − 40 px so the pill div
          // can fully fill the OS window's interior at the cap). The
          // ResizeObserver above watches `pillRef.contentRect.width`
          // and pushes that back to Rust so the OS window grows in
          // lockstep, and also drives the dynamic min-width lerp so
          // tall content forces the box wider rather than narrower.
          width: STREAM_GROW_ENABLED() ? "fit-content" : "116px",
          // Streaming uses the floor signal so the ResizeObserver-driven
          // width-floor lerp survives Solid re-renders triggered by
          // unrelated style changes (e.g. box-shadow flipping on Done).
          // 0 sentinel = floor untouched, fall back to the static W_MIN.
          "min-width": STREAM_GROW_ENABLED()
            ? `${Math.max(W_MIN, dynamicMinWidth())}px`
            : "116px",
          "max-width": STREAM_GROW_ENABLED() ? `${W_MAX}px` : "116px",
          // Clip streaming-mode children to the rounded boundary.
          // Without this, tall pills painted square corners because
          // StreamingText / ShimmerWord painted past the rim radius.
          // Restricted to streaming mode so the QueueDot (which
          // anchors at right:-3px in compact mode) stays visible —
          // box-shadow / inner-rim glows live OUTSIDE the box and
          // are unaffected either way.
          overflow: STREAM_GROW_ENABLED() ? "hidden" : "visible",
          // Multi-line streaming makes a pure pill (border-radius:
          // 9999px) look squashed at the corners — soften to a 14 px
          // rounded rect when wide and let the compact mode keep its
          // round-end silhouette via a higher value.
          // When the mode drawer is attached below, flatten the bottom
          // corners so the pill and drawer share a seamless continuous edge.
          "border-radius": STREAM_GROW_ENABLED()
            ? "14px"
            : modeChip()
              ? "9999px 9999px 0 0"
              : "9999px",
          background: "var(--color-pill-surface)",
          "backdrop-filter": "var(--glass-blur)",
          "-webkit-backdrop-filter": "var(--glass-blur)",
          border: "1px solid var(--glass-rim)",
          "box-shadow": isDoneSuccess()
            ? "inset 0 0 0 1.5px var(--color-success), 0 0 18px var(--pill-glow-success), var(--glass-shadow-pill)"
            : isDoneWarning()
              ? "inset 0 0 0 1.5px var(--color-warning), 0 0 18px var(--pill-glow-warning), var(--glass-shadow-pill)"
              : isSaturated()
                ? "inset 0 0 0 1.5px var(--color-warning), 0 0 18px var(--pill-glow-saturated), var(--glass-shadow-pill)"
                : "var(--glass-shadow-pill)",
        }}
        role="status"
        aria-live="off"
      >
        {/* Stage-only live region. The pill body mutates per token
            during streaming, which would spam screen readers if the
            outer container were polite. We keep the outer container
            silent (aria-live="off") and announce stage transitions
            (Listening/Transcribing/Polishing/Done) through this
            visually-hidden child region instead. */}
        <span
          aria-live="polite"
          aria-atomic="true"
          style={{
            position: "absolute",
            width: "1px",
            height: "1px",
            padding: 0,
            margin: "-1px",
            overflow: "hidden",
            clip: "rect(0, 0, 0, 0)",
            "white-space": "nowrap",
            border: 0,
          }}
        >
          {(() => {
            const hud = modeHud();
            if (hud) return `Mode: ${hud.name}`;
            const st = effectiveState();
            if (st === "listening") return "Listening";
            if (st === "transcribing") return "Transcribing";
            if (st === "polishing") return "Polishing";
            if (st === "done") return message() ?? (warning() ? "Raw copy" : "Done");
            if (st === "error") return message() ?? "Error";
            if (st === "canceled") return message() ?? "Canceled";
            if (st === "saturated") return toastMessage() ?? message() ?? "Too many in flight";
            return "";
          })()}
        </span>
        <Show when={modeHud()}>
          <ModeHudView mode={modeHud()!} />
        </Show>
        <Show when={state() === "listening" && !modeHud()}>
          <Waveform levels={levels()} color={meterColor()} />
        </Show>

        {/* Unified processing layout: sticky header + scrollable body.
            Active for both transcribing and polishing so the stage
            label never scrolls off as text grows. */}
        <Show when={isProcessing() && !modeHud()}>
          {/* Sticky header — always visible for the entire stage. */}
          <div class="processing-header" style={{ "padding-bottom": "2px" }}>
            <Show when={effectiveState() === "polishing"}>
              <PolishingLabelRow visible={true} />
            </Show>
            <Show when={effectiveState() === "transcribing"}>
              <TranscribingLabelRow />
            </Show>
          </div>

          {/* Scrollable body: auto-scrolls to bottom, fades top edge,
              never grows past H_MAX minus header height. */}
          <div
            ref={(el) => (streamingBodyRef = el)}
            class="streaming-body"
            style={{
              flex: "1",
              "max-height": `${H_MAX - 28}px`,
              "mask-image": bodyOverflowing()
                ? "linear-gradient(to bottom, transparent 0%, black 24px)"
                : "none",
              "-webkit-mask-image": bodyOverflowing()
                ? "linear-gradient(to bottom, transparent 0%, black 24px)"
                : "none",
            }}
          >
            <Show when={isStreaming()}>
              <StreamingText
                text={streamingText()}
                mode={streamingMode() ?? "transcript"}
                waitingForPolish={isWaitingForPolish()}
                transcribingStreaming={
                  effectiveState() === "transcribing" &&
                  streamingMode() === "transcript"
                }
              />
            </Show>
            <Show when={!isStreaming()}>
              <TypingDots color={meterColor()} />
            </Show>
          </div>
        </Show>

        <Show when={state() === "done" && !modeHud()}>
          <TerminalLine
            tone={warning() ? "warning" : "success"}
            label={message() ?? (warning() ? "Raw copy" : "Done")}
          />
        </Show>

        <Show when={state() === "error" && !modeHud()}>
          <TerminalLine
            tone="error"
            label={message() ?? "Error"}
          />
        </Show>

        <Show when={state() === "canceled" && !modeHud()}>
          <TerminalLine tone="muted" label={message() ?? "Canceled"} />
        </Show>

        <Show when={isSaturated() && !modeHud()}>
          <TerminalLine
            tone="warning"
            label={toastMessage() ?? message() ?? "Too many in flight"}
          />
        </Show>

        <Show when={backgroundCount() > 0 && !isSaturated() && !modeHud()}>
          <QueueDot count={backgroundCount()} />
        </Show>
      </div>

      {/* Mode drawer: attached half-pill that grows downward from the
          main pill when the mode-cycle hotkey fires during a recording
          session. No gap — the flat bottom edge of the main pill and
          the flat top edge of the drawer share a seamless seam. */}
      <ModeDrawer mode={modeChip()} fading={modeChipFading()} />
      </div>
    </div>
  );
}

function clamp(n: number): number {
  if (Number.isNaN(n)) return 0;
  return Math.max(0, Math.min(1, n));
}

function ModeHudView(props: { mode: ActiveModePayload }) {
  return (
    <div
      class="flex h-full w-full items-center justify-center gap-1.5 px-1 text-[12px] font-medium leading-none text-[var(--color-ink)]"
      aria-hidden="true"
    >
      <span class="shrink-0">{props.mode.icon}</span>
      <span class="truncate">{props.mode.name}</span>
    </div>
  );
}

function Waveform(props: { levels: number[]; color: string }) {
  // Waveform spans the full pill width; the edge-envelope opacity is
  // what the eye reads as "the wave fades into the pill" instead of a
  // hard cluster floating in the middle with visible empty padding on
  // the sides.
  return (
    <div
      class="flex h-full w-full items-center justify-between"
      aria-hidden="true"
    >
      {props.levels.map((level, i) => {
        // Gamma 0.65 opens up the bottom of the range — Whisper-grade
        // RMS spends most of its life in [0, 0.3].
        const shaped = Math.pow(level, 0.65);
        // Sine bell across the bar index. 0 at edges, 1 at the
        // middle — both height and opacity ride this so the wave
        // visually disappears into the pill rim.
        const env = edgeEnvelope(i, props.levels.length);
        const h = Math.max(
          1.5,
          Math.round((0.22 + shaped * 0.78) * 18 * (0.4 + 0.6 * env)),
        );
        const opacity = (0.32 + shaped * 0.68) * env;
        return (
          <div
            style={{
              width: "1.5px",
              height: `${h}px`,
              "border-radius": "1.5px",
              background: props.color,
              opacity,
              transition: "height 110ms ease-out, opacity 180ms ease-out",
            }}
          />
        );
      })}
    </div>
  );
}

// Soft sine bell. Returns ~0 at i=0 and i=n-1, peaks at 1 in the
// middle. Used as an envelope on both height and opacity so the
// waveform fades into the pill rim.
function edgeEnvelope(i: number, n: number): number {
  const t = (i + 0.5) / n;
  return Math.sin(Math.PI * t);
}

/// Single-slot streaming text. Wraps to multiple lines so long
/// transcripts and polishes are not cropped to a single 26px ribbon;
/// the parent pill window grows in lockstep via the ResizeObserver
/// hook in PillView.
///
/// Four visual states share this slot:
///   1. Transcript streaming: cyan caret + glow blinking after text,
///      and a continuous cyan `pill-shimmer` sweep across the body
///      itself so the active transcription reads as alive — not just
///      the trailing caret. (Brief: "I want to always see this cyan
///      shimmer, which goes from left to right during transcription,
///      in addition to the existing caret".)
///   2. Waiting for polish:   `pill-shimmer` sweep recolours to
///      violet — the transcript freeze between final-transcript and
///      the LLM's first token gets the polish tint so the gap reads
///      as active processing instead of a frozen pill.
///   3. Polish streaming:     violet caret + glow, plain ink text.
///   4. (Reduced motion):     CSS overrides drop the shimmer
///      animation and hand the body back a solid ink colour, and the
///      caret freezes solid. This file doesn't need to branch on it —
///      the shimmer/caret CSS classes both opt out under
///      prefers-reduced-motion.
function StreamingText(props: {
  text: string;
  mode: "transcript" | "polish";
  waitingForPolish: boolean;
  /// True while we are actively in the `transcribing` stage with a
  /// transcript stream still flowing (i.e. *before* the polish stream
  /// starts). Drives the cyan body shimmer. Mutually exclusive with
  /// `waitingForPolish` by construction in the parent (waitingForPolish
  /// requires `effectiveState === "polishing"`, transcribingStreaming
  /// requires `effectiveState === "transcribing"`).
  transcribingStreaming: boolean;
}) {
  // Caret colour: violet during the polish stage (and during the
  // transcript→polish handoff via `waitingForPolish`), cyan during the
  // active transcribing stage. Cyan was specifically called out by
  // design so the transcribing stage no longer reuses the teal accent.
  const caretColor = () =>
    props.mode === "polish" ? "var(--color-violet)" : "var(--color-cyan)";
  // Glow points at the theme-aware token so light vs dark themes pick
  // up rgbas tuned for each surface — the previous hardcoded rgba
  // strings produced an off-hue ring on the cream pill in light mode.
  const caretGlow = () =>
    props.mode === "polish"
      ? "var(--caret-glow-violet)"
      : "var(--caret-glow-cyan)";
  // Shimmer is active throughout both transcribing and polishing streaming
  // phases so the user always sees the animated sweep on live text.
  const shimmerActive = () =>
    props.waitingForPolish ||
    props.transcribingStreaming ||
    props.mode === "polish";
  const shimmerTint = () =>
    props.waitingForPolish || props.mode === "polish"
      ? "var(--color-violet)"
      : "var(--color-cyan)";
  return (
    <div
      class="flex w-full select-none items-start"
      style={{
        // Left-aligned for multi-line readability. Right alignment
        // works for one-liners but flips reading direction at every
        // wrap, hostile for anything past ~10 words.
        "justify-content": "flex-start",
      }}
      aria-live="off"
    >
      <span
        class="text-[11px] font-medium"
        classList={{ "pill-shimmer": shimmerActive() }}
        style={{
          color: "var(--color-ink)",
          "letter-spacing": "0.005em",
          opacity: 0.95,
          // 1.4 line-height at 11px text yields ~15px per line; a
          // 5-line polish lands in ~75px of pill height.
          "line-height": "1.4",
          // pre-wrap (not normal) so LLM-polished text that contains
          // \n\n paragraphs, numbered lists, or bullet points keeps
          // its line breaks instead of collapsing to a single visual
          // line. Wrapping behaviour stays the same — soft wrap at
          // word boundaries, with `word-break: break-word` to break
          // absurdly long tokens (pasted URLs) before they escape
          // the right rim. The pill's ResizeObserver picks up the
          // new height naturally as multi-line content expands and
          // pushes a `pill_resize` to grow the OS window in lockstep.
          "white-space": "pre-wrap",
          "word-break": "break-word",
          padding: "2px 0",
          width: "100%",
          transition: "color 180ms ease-out",
          // Shimmer ramp: ink -> tint -> ink. Tint flips to cyan
          // during transcription streaming and violet during the
          // polish-gap, so the band visibly carries the stage colour.
          "--shimmer-base": "var(--color-ink)",
          "--shimmer-tint": shimmerTint(),
        }}
      >
        {props.text || " "}
        {/* Inline blinking caret stays attached to the trailing
            character so on multi-line wraps it sits at the end of
            the last line. Hidden during the polish-gap shimmer
            because the shimmer itself is the activity indicator —
            but stays VISIBLE during transcribing-streaming shimmer,
            since the caret marks where the next token will land. */}
        <Show when={!props.waitingForPolish}>
          <span
            class="streaming-caret"
            aria-hidden="true"
            style={{
              "background-color": caretColor(),
              "margin-left": "2px",
              "margin-right": "1px",
              "vertical-align": "text-bottom",
              display: "inline-block",
              width: "2px",
              height: "16px",
              "border-radius": "1px",
              // CSS class .streaming-caret reads --caret-glow for
              // its box-shadow, so flipping the variable per-state
              // gets us the cyan/violet halo without animation forks.
              "--caret-glow": caretGlow(),
            }}
          />
        </Show>
      </span>
    </div>
  );
}

/// Persistent "Polishing…" label row. Sits in normal flow above the
/// streaming body when the parent pill is in column layout (gated on
/// `state() === "polishing"`). Visible for the whole polishing stage;
/// crossfades in/out via CSS opacity transition.
function PolishingLabelRow(props: { visible: boolean }) {
  return (
    <div
      class="polishing-label-row pointer-events-none"
      style={{
        display: "flex",
        "align-items": "center",
        gap: "6px",
        // 4 px gap to the streaming body underneath, per design spec.
        "margin-bottom": "4px",
        opacity: props.visible ? 1 : 0,
        transition: "opacity 180ms ease-out",
      }}
      aria-hidden="true"
    >
      <span
        class="polishing-dot"
        style={{
          width: "6px",
          height: "6px",
          "border-radius": "9999px",
          background: "var(--color-violet)",
          // Theme-aware halo via the --polishing-dot-glow token so the
          // light/dark themes each get a hue + alpha tuned for their
          // pill surface — replaces the previously hardcoded rgba.
          "box-shadow": "0 0 8px var(--polishing-dot-glow)",
          display: "inline-block",
        }}
      />
      <span
        style={{
          "font-size": "10.5px",
          "font-weight": 600,
          "letter-spacing": "0.04em",
          color: "var(--color-violet)",
          opacity: 0.85,
          "line-height": "1",
        }}
      >
        Polishing…
      </span>
    </div>
  );
}

/// Sticky header for the transcribing stage — mirrors PolishingLabelRow
/// with cyan instead of violet.
function TranscribingLabelRow() {
  return (
    <div
      class="polishing-label-row pointer-events-none"
      style={{
        display: "flex",
        "align-items": "center",
        gap: "6px",
        "margin-bottom": "4px",
        opacity: 1,
        transition: "opacity 180ms ease-out",
      }}
      aria-hidden="true"
    >
      <span
        class="polishing-dot"
        style={{
          width: "6px",
          height: "6px",
          "border-radius": "9999px",
          background: "var(--color-cyan)",
          "box-shadow": "0 0 8px var(--caret-glow-cyan)",
          display: "inline-block",
        }}
      />
      <span
        style={{
          "font-size": "10.5px",
          "font-weight": 600,
          "letter-spacing": "0.04em",
          color: "var(--color-cyan)",
          opacity: 0.85,
          "line-height": "1",
        }}
      >
        Transcribing…
      </span>
    </div>
  );
}

/// Three staggered pulsing dots shown in the body area while no streamed
/// text has arrived yet. Gives a visible loading signal during the
/// Whisper startup gap and the LLM TTFT gap.
function TypingDots(props: { color: string }) {
  const delays = ["0ms", "120ms", "240ms"];
  return (
    <div
      class="flex items-center justify-center"
      style={{ gap: "5px", padding: "4px 0 2px" }}
      aria-hidden="true"
    >
      {delays.map((delay) => (
        <span
          class="typing-dot"
          style={{
            width: "5px",
            height: "5px",
            "border-radius": "9999px",
            background: props.color,
            display: "inline-block",
            "animation-delay": delay,
          }}
        />
      ))}
    </div>
  );
}

/// Mode drawer: an "attached half-pill" that grows downward from the bottom
/// of the main listening pill when the mode-cycle hotkey fires during a
/// recording session. No gap between pill and drawer — they share a continuous
/// flat seam. The main pill's bottom border-radius transitions to 0 in lockstep.
///
/// Open:  height 0→25px + opacity 0→1 over 220ms ease-out.
/// Close: fading=true drives opacity 1→0 over 220ms, then the parent
///        clears `mode` (unmounting the content) after the same 220ms timer.
///
/// `mode` null means nothing is mounted — the drawer collapses to 0 height.
function ModeDrawer(props: { mode: ActiveModePayload | null; fading: boolean }) {
  return (
    <div
      class="pointer-events-none overflow-hidden"
      style={{
        // Grow downward from height 0. The 25px target matches the
        // "half-pill" spec: padding (3px top/bottom) + content (~11px)
        // + border-bottom (1px) = ~25px. Use height transition (not
        // scaleY) so the rounded bottom corners aren't distorted.
        height: props.mode ? "25px" : "0px",
        opacity: props.mode && !props.fading ? 1 : 0,
        transition: "height 220ms ease-out, opacity 220ms ease-out",
        // Full width of the wrapper so longer mode names (e.g. "Code Comment")
        // aren't clipped; visual distinction from the main pill comes from the
        // height difference and the rounded-bottom-only shape.
        width: "100%",
        // Flat top edge (shared seam with pill), rounded bottom corners.
        // 18px mirrors the effective radius on the main pill's top corners
        // (border-radius:9999px on 116×36px → min(58,18) = 18px).
        "border-radius": "0 0 18px 18px",
        background: "var(--color-pill-surface)",
        "backdrop-filter": "var(--glass-blur)",
        "-webkit-backdrop-filter": "var(--glass-blur)",
        // No top border — the shared seam must be invisible.
        "border-left": "1px solid var(--glass-rim)",
        "border-right": "1px solid var(--glass-rim)",
        "border-bottom": "1px solid var(--glass-rim)",
      }}
      aria-hidden="true"
    >
      <Show when={props.mode}>
        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "center",
            gap: "5px",
            padding: "3px 8px",
            "font-size": "10.5px",
            "font-weight": 500,
            color: "var(--color-ink-mute)",
            "white-space": "nowrap",
          }}
        >
          <span>{props.mode!.icon}</span>
          <span>{props.mode!.name}</span>
        </div>
      </Show>
    </div>
  );
}


function TerminalLine(props: {
  tone: "success" | "warning" | "error" | "muted";
  label: string;
}) {
  return (
    <div class="flex items-center gap-1.5">
      <Show when={props.tone === "success"}>
        <CheckIcon />
      </Show>
      <Show when={props.tone === "warning"}>
        <WarningIcon />
      </Show>
      <Show when={props.tone === "error"}>
        <AlertIcon />
      </Show>
      <Show when={props.tone === "muted"}>
        <XIcon />
      </Show>
      <span
        class="text-[11px] font-medium leading-none"
        style={{
          color:
            props.tone === "success"
              ? "var(--color-success)"
              : props.tone === "warning"
                ? "var(--color-warning)"
                : props.tone === "error"
                  ? "var(--color-danger)"
                  : "var(--color-ink-mute)",
        }}
      >
        {props.label}
      </span>
    </div>
  );
}

/// Small violet circle anchored to the right edge of the pill. Shows
/// a count when 2 or more background sessions are running; just a
/// dot otherwise. Sits on top of the pill content via absolute
/// positioning so it never reflows the foreground text.
function QueueDot(props: { count: number }) {
  return (
    <div
      class="pointer-events-none absolute"
      style={{
        right: "-3px",
        top: "50%",
        transform: "translateY(-50%)",
        width: "9px",
        height: "9px",
        "border-radius": "9999px",
        background: "var(--color-violet, #a78bfa)",
        // Halo ring matches the surrounding bg, not the translucent pill
        // surface — using --color-pill-surface in light mode produced a
        // visible cream halo around the violet dot that read as a paint
        // bug. --color-bg merges into the ambient backdrop at any theme.
        "box-shadow": "0 0 0 1.5px var(--color-bg), 0 0 8px rgba(167, 139, 250, 0.55)",
        display: "flex",
        "align-items": "center",
        "justify-content": "center",
      }}
      aria-hidden="true"
      title={`${props.count} background session${props.count === 1 ? "" : "s"}`}
    >
      <Show when={props.count >= 2}>
        <span
          style={{
            "font-size": "7px",
            "font-weight": "700",
            color: "white",
            "line-height": "1",
          }}
        >
          {props.count}
        </span>
      </Show>
    </div>
  );
}

function CheckIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-success)"
      stroke-width="2.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <polyline points="4 12 10 18 20 6" />
    </svg>
  );
}

function AlertIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-danger)"
      stroke-width="2.25"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="9" />
      <line x1="12" y1="8" x2="12" y2="12" />
      <line x1="12" y1="16" x2="12.01" y2="16" />
    </svg>
  );
}

function WarningIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-warning)"
      stroke-width="2.25"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M12 3 L22 20 L2 20 Z" />
      <line x1="12" y1="10" x2="12" y2="14" />
      <line x1="12" y1="17" x2="12.01" y2="17" />
    </svg>
  );
}

function XIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-ink-mute)"
      stroke-width="2.25"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <line x1="6" y1="6" x2="18" y2="18" />
      <line x1="6" y1="18" x2="18" y2="6" />
    </svg>
  );
}
