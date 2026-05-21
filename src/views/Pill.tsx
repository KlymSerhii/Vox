import {
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  getPublicSettings,
  onSettingsChanged,
  pillResize,
  type Settings,
} from "../lib/ipc";
import {
  BARS,
  H_MAX,
  W_MAX,
  W_MIN,
  WINDOW_PADDING_H,
  WINDOW_PADDING_W,
} from "./pill/constants";
import { clamp } from "./pill/helpers";
import {
  type ActiveModePayload,
  type AudioLevelPayload,
  type PillState,
  type PillStatePayload,
  type StreamPayload,
  type ToastPayload,
} from "./pill/types";
import { useTranscriptHold } from "./pill/useTranscriptHold";
import { useReveal } from "./pill/useReveal";
import { useModeHud } from "./pill/useModeHud";
import { usePillResize } from "./pill/usePillResize";
import { ModeHudView } from "./pill/components/ModeHudView";
import { Waveform } from "./pill/components/Waveform";
import {
  ListeningLabelRow,
  PolishingLabelRow,
  TranscribingLabelRow,
} from "./pill/components/StageLabels";
import { StreamingText } from "./pill/components/StreamingText";
import { TypingDots } from "./pill/components/TypingDots";
import { TerminalLine } from "./pill/components/TerminalLine";
import { QueueDot } from "./pill/components/QueueDot";
import { ModeDrawer } from "./pill/components/ModeDrawer";

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

  // Ref to the scrollable streaming body — used by the auto-scroll effect.
  let streamingBodyRef: HTMLDivElement | undefined;
  // True only when the body has actual overflow (old content has scrolled
  // out of view). Gates the top-fade mask so TypingDots / short text stay
  // fully visible until there is genuine content to hide.
  const [bodyOverflowing, setBodyOverflowing] = createSignal(false);

  // Opt-in toggle from Settings → General / tray popover. When true the
  // pill paints the speculative transcript while the user is
  // still speaking, in place of the default waveform-only listening
  // silhouette. Settings load is async — `false` until the first
  // `getPublicSettings()` lands, which matches the default and means
  // a user who hasn't opted in never sees a flash of the wide layout.
  const [liveTranscriptEnabled, setLiveTranscriptEnabled] =
    createSignal(false);
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

  let unlisten: UnlistenFn[] = [];
  let toastTimer: number | null = null;

  // ─── Composed state ────────────────────────────────────────────────
  // Each hook owns its own signals, timers, and onCleanup. Wiring order
  // matters: useReveal depends on the transcript-hold callbacks, and
  // useModeHud reads `sessionActive`.
  const transcriptHold = useTranscriptHold();
  const { isTranscriptHoldActive } = transcriptHold;
  const reveal = useReveal({
    reducedMotion,
    state,
    liveTranscriptEnabled,
    clearTranscriptHold: transcriptHold.clearTranscriptHold,
    scheduleTranscriptHold: transcriptHold.scheduleTranscriptHold,
  });
  const {
    streamingText,
    streamingMode,
    applyStreamPayload,
    kickSlowRevealForTranscribing,
    clearStreaming,
  } = reveal;
  const { modeHud, modeChip, modeChipFading, clearModeHud, clearModeChip, showMode } =
    useModeHud({ sessionActive });
  const {
    setWrapperRef,
    setPillRef,
    dynamicMinWidth,
    setupResizeObserver,
    teardownResizeObserver,
  } = usePillResize();

  // Apply live-transcript-relevant fields from a Settings snapshot.
  // The tray popover / Settings UI broadcast `settings:changed` after
  // every flip so this stays in lockstep without a Vox restart.
  function applySettingsToPill(next: Settings) {
    setLiveTranscriptEnabled(!!next.live_transcript_enabled);
  }

  onMount(async () => {
    document.body.classList.add("translucent");

    // First read is async so the pill paints in its default
    // (compact, waveform) silhouette until settings land — matches
    // what a user who never flipped the toggle will always see.
    // Failure is non-fatal: the pill stays in default mode.
    try {
      const initial = await getPublicSettings();
      applySettingsToPill(initial);
    } catch (e) {
      console.warn("pill: getPublicSettings failed:", e);
    }
    unlisten.push(await onSettingsChanged(applySettingsToPill));

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
        // the speculative buffer would surface. The reveal hook owns
        // the slow-walk pacing + guards; we only signal the edge here.
        if (prevState === "listening" && e.payload.state === "transcribing") {
          kickSlowRevealForTranscribing();
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
        showMode(e.payload);
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
    if (reducedMotionMql && reducedMotionListener) {
      reducedMotionMql.removeEventListener("change", reducedMotionListener);
      reducedMotionMql = null;
      reducedMotionListener = null;
    }
  });

  const isDone = () => state() === "done";
  const isDoneWarning = () => isDone() && warning();
  const isDoneSuccess = () => isDone() && !warning();
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
  // True only when the user opted in AND we're actively in the
  // listening phase. The reduced-motion bail in `applyStreamPayload`
  // already drops speculative chunks, so we gate the wide layout on
  // the same flag here — a reduced-motion user keeps the compact
  // waveform silhouette during listening even with the toggle on.
  const isLiveListening = () =>
    state() === "listening" && liveTranscriptEnabled() && !reducedMotion();
  // Body slot shows StreamingText whenever there's actual text to
  // paint; OR otherwise gates to TypingDots inside the same body.
  // Broadened to cover the live-listening case so the body switches
  // on as soon as the first speculative chunk arrives.
  const isStreaming = () =>
    (isProcessing() || isLiveListening()) &&
    streamingMode() !== null &&
    streamingText().length > 0;
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

  // Wide layout active during processing AND during live-listening so
  // the ResizeObserver / dynamic min-width / column flexbox all
  // engage as soon as the listening phase opens, instead of waiting
  // for the listening → transcribing flip.
  const STREAM_GROW_ENABLED = () =>
    isStreaming() || isWaitingForPolish() || isLiveListening();

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
  //
  // `wasStreaming` doubles as the edge-trigger guard: the createEffect
  // re-fires on every streamingText / streamingMode change while the
  // wide layout is up (because those are tracked dependencies of
  // STREAM_GROW_ENABLED). Without this guard the pre-emptive widen
  // would re-fire ~30 times/sec during the reveal, racing with the
  // ResizeObserver's content-driven pillResize and snapping the OS
  // window back to 196 px every tick — visibly clipping the right
  // edge of any transcript wider than 156 px of interior.
  let wasStreaming = false;
  createEffect(() => {
    const streamingNow = STREAM_GROW_ENABLED();
    if (streamingNow) {
      setupResizeObserver();
      // Pre-emptive widen ONLY on the false→true edge so the OS
      // window catches up to the CSS layout in the same frame the
      // wide silhouette renders. Without this, transitioning into
      // the wide layout from a compact 116 px OS window would clip
      // the inner pill (which is already laid out at the 180 px
      // floor) for one frame. Live-listening relies on this path
      // entirely — Rust no longer widens on session-begin since it
      // has no view of the user's reduced-motion preference. The
      // ResizeObserver follows immediately with the real content
      // size and is the only thing that should resize the window
      // after this initial widen.
      if (!wasStreaming) {
        wasStreaming = true;
        void pillResize({
          width: W_MIN + WINDOW_PADDING_W,
        }).catch(() => { });
      }
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
        }).catch(() => { });
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
      void pillResize({ height: 72 }).catch(() => { });
    } else if (!fading) {
      // Drawer gone — restore compact height.
      void pillResize({ height: 36 + WINDOW_PADDING_H }).catch(() => { });
    }
  });

  return (
    <div class="flex h-screen w-screen items-center justify-center bg-transparent">
      <div ref={setWrapperRef} class="flex flex-col items-center">
        <div
          ref={setPillRef}
          class="animate-pill-in pointer-events-none relative flex min-h-[26px] items-center rounded-[14px] px-3 py-1.5 transition-[width,box-shadow,padding,border-radius] duration-200 ease-out"
          classList={{ "pill-transitioning": pillTransitioning() }}
          style={{
            // Column layout during any wide-layout stage so the sticky
            // header sits above the scrollable streaming body. Other
            // states stay row-centred. Live-listening shares the same
            // header + body shape as transcribing/polishing, so it
            // has to share the column flex direction too — gating
            // only on `isProcessing()` left the listening header and
            // body sitting horizontally and clipping each other.
            "flex-direction":
              isProcessing() || isLiveListening() ? "column" : "row",
            "justify-content":
              isProcessing() || isLiveListening() ? "flex-start" : "center",
            "align-items":
              isProcessing() || isLiveListening() ? "stretch" : "center",
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
            // corners so the pill and drawer share a seamless edge.
            // Live-listening can host the mode chip too (the user can
            // still cycle modes mid-utterance), so the bottom-flatten
            // branch has to win over the wide-rounded-rect branch when
            // both are active.
            "border-radius": modeChip()
              ? STREAM_GROW_ENABLED()
                ? "14px 14px 0 0"
                : "9999px 9999px 0 0"
              : STREAM_GROW_ENABLED()
                ? "14px"
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
              if (st === "listening") {
                return isLiveListening()
                  ? "Listening — live transcript"
                  : "Listening";
              }
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
          {/* Waveform is the default listening silhouette. Suppressed
            when the live-transcript opt-in is on — that path renders
            the wide layout below with the caption body instead. */}
          <Show when={state() === "listening" && !modeHud() && !isLiveListening()}>
            <Waveform levels={levels()} color={meterColor()} />
          </Show>

          {/* Unified wide layout: sticky header + scrollable body.
            Active for transcribing and polishing always, and for
            listening when the user opted into live transcript so the
            speculative caption renders in the same slot the post-stop
            reveal will inherit (no layout reflow on hotkey release). */}
          <Show when={(isProcessing() || isLiveListening()) && !modeHud()}>
            {/* Sticky header — always visible for the entire stage. */}
            <div class="processing-header" style={{ "padding-bottom": "2px" }}>
              <Show when={effectiveState() === "polishing"}>
                <PolishingLabelRow visible={true} />
              </Show>
              <Show when={effectiveState() === "transcribing"}>
                <TranscribingLabelRow />
              </Show>
              <Show when={isLiveListening() && effectiveState() === "listening"}>
                <ListeningLabelRow />
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
                  listening={isLiveListening()}
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
