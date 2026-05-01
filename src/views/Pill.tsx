import { createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

const BARS = 32;
/** Soft cap on how many characters per second the streaming UI
 * reveals. Above ~40 cps the eye can't follow individual edits and
 * the pill flickers with each token. Tuned empirically for Gemma's
 * ~50 tok/s on Apple Silicon. */
const REVEAL_CHARS_PER_SEC = 40;
const REVEAL_TICK_MS = 33; // ~30 Hz

export default function PillView() {
  const [state, setState] = createSignal<PillState>("listening");
  const [message, setMessage] = createSignal<string | null>(null);
  const [warning, setWarning] = createSignal(false);
  const [levels, setLevels] = createSignal<number[]>(new Array(BARS).fill(0));
  const [backgroundCount, setBackgroundCount] = createSignal(0);
  const [toastMessage, setToastMessage] = createSignal<string | null>(null);

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

  function startReveal(initialVisibleLen: number) {
    stopReveal();
    revealStartedAt = performance.now();
    revealStartLen = initialVisibleLen;
    revealTimer = window.setInterval(() => {
      const elapsedSec = (performance.now() - revealStartedAt) / 1000;
      const allowedLen = Math.floor(
        revealStartLen + REVEAL_CHARS_PER_SEC * elapsedSec,
      );
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

    // First polish chunk after a transcript stream — wipe the slate so
    // the polish text replaces the transcript instead of stacking.
    if (streamingMode() !== mode) {
      setStreamingMode(mode);
      setStreamingText("");
      revealStartLen = 0;
    }

    targetText = payload.text;

    if (payload.final) {
      flushReveal();
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
        setState(e.payload.state);
        setMessage(e.payload.message ?? null);
        setWarning(e.payload.warning ?? false);
        setBackgroundCount(e.payload.background_count ?? 0);
        // Listening starts a brand-new utterance — drop any leftover
        // streaming buffers from a stale session that finalised.
        if (e.payload.state === "listening") {
          clearStreaming();
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
  });

  onCleanup(() => {
    for (const u of unlisten) u();
    if (toastTimer !== null) {
      clearTimeout(toastTimer);
    }
    stopReveal();
    if (reducedMotionMql && reducedMotionListener) {
      reducedMotionMql.removeEventListener("change", reducedMotionListener);
      reducedMotionMql = null;
      reducedMotionListener = null;
    }
  });

  const isDone = () => state() === "done";
  const isDoneWarning = () => isDone() && warning();
  const isDoneSuccess = () => isDone() && !warning();
  const isProcessing = () =>
    state() === "transcribing" || state() === "polishing";
  const isSaturated = () => state() === "saturated";
  const isStreaming = () =>
    isProcessing() && streamingMode() !== null && streamingText().length > 0;

  // Listening swaps the meter to warm coral so the brand color earns
  // its keep at the one moment the device is recording. DESIGN.md §5.4.
  const meterColor = createMemo(() => {
    switch (state()) {
      case "listening":
        return "var(--color-coral)";
      case "transcribing":
        return "var(--color-accent)";
      case "polishing":
        return "var(--color-violet)";
      default:
        return "var(--color-meter)";
    }
  });

  return (
    <div class="flex h-screen w-screen items-center justify-center bg-transparent">
      <div
        class="animate-pill-in pointer-events-none relative flex h-[26px] items-center justify-center rounded-full px-3 transition-[width,box-shadow,padding] duration-200 ease-out"
        style={{
          width: isStreaming() ? "calc(100vw - 32px)" : "116px",
          "max-width": "608px",
          background: "var(--color-pill-surface)",
          "backdrop-filter": "var(--glass-blur)",
          "-webkit-backdrop-filter": "var(--glass-blur)",
          border: "1px solid var(--glass-rim)",
          "box-shadow": isDoneSuccess()
            ? "inset 0 0 0 1.5px var(--color-success), 0 0 18px rgba(74, 222, 128, 0.42), var(--glass-shadow-pill)"
            : isDoneWarning()
              ? "inset 0 0 0 1.5px var(--color-warning), 0 0 18px rgba(251, 191, 36, 0.38), var(--glass-shadow-pill)"
              : isSaturated()
                ? "inset 0 0 0 1.5px var(--color-warning), 0 0 18px rgba(251, 191, 36, 0.32), var(--glass-shadow-pill)"
                : "var(--glass-shadow-pill)",
        }}
        role="status"
        aria-live="polite"
      >
        <Show when={state() === "listening"}>
          <Waveform levels={levels()} color={meterColor()} />
        </Show>

        <Show when={isProcessing() && !isStreaming()}>
          <ShimmerWord
            text={state() === "transcribing" ? "Transcribing" : "Polishing"}
            tint={meterColor()}
          />
        </Show>

        <Show when={isStreaming()}>
          <StreamingText
            text={streamingText()}
            mode={streamingMode() ?? "transcript"}
          />
        </Show>

        <Show when={state() === "done"}>
          <TerminalLine
            tone={warning() ? "warning" : "success"}
            label={message() ?? (warning() ? "Raw copy" : "Done")}
          />
        </Show>

        <Show when={state() === "error"}>
          <TerminalLine
            tone="error"
            label={message() ?? "Error"}
          />
        </Show>

        <Show when={state() === "canceled"}>
          <TerminalLine tone="muted" label={message() ?? "Canceled"} />
        </Show>

        <Show when={isSaturated()}>
          <TerminalLine
            tone="warning"
            label={toastMessage() ?? message() ?? "Too many in flight"}
          />
        </Show>

        <Show when={backgroundCount() > 0 && !isSaturated()}>
          <QueueDot count={backgroundCount()} />
        </Show>
      </div>
    </div>
  );
}

function clamp(n: number): number {
  if (Number.isNaN(n)) return 0;
  return Math.max(0, Math.min(1, n));
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

/// Single-slot streaming text. Mode toggles change the tint so the
/// transcript→polish handoff is visible without stacking two blocks.
/// Long text scrolls horizontally via overflow:hidden and a "tail"
/// alignment so the latest chars sit closest to the right edge.
function StreamingText(props: {
  text: string;
  mode: "transcript" | "polish";
}) {
  const tint = () =>
    props.mode === "polish" ? "var(--color-violet)" : "var(--color-accent)";
  return (
    <div
      class="flex h-full w-full select-none items-center overflow-hidden"
      style={{
        // Right-aligned text so the most recently appended characters
        // sit beside the right rim — that's where the eye reads
        // forward and matches how a streaming console feels.
        "justify-content": "flex-end",
      }}
      aria-live="polite"
    >
      <span
        class="whitespace-nowrap text-[11px] font-medium leading-none"
        style={{
          color: "var(--color-ink)",
          "letter-spacing": "0.005em",
          opacity: 0.95,
          transition: "color 180ms ease-out",
          "border-right": `2px solid ${tint()}`,
          "padding-right": "6px",
        }}
      >
        {props.text || " "}
      </span>
    </div>
  );
}

function ShimmerWord(props: { text: string; tint: string }) {
  // A continuous left-to-right luminance sweep across the letters. The
  // base glyphs sit at ink-mute; a brighter band travels through them
  // on a 1.6s loop to communicate active progress without the visual
  // noise of a spinner. CSS variables drive the gradient so the
  // shimmer recolours per-state (teal → violet) to match the pipeline
  // stage.
  return (
    <span
      class="pill-shimmer select-none text-[10.5px] font-medium leading-none"
      style={{
        "--shimmer-base": "var(--color-ink-mute)",
        "--shimmer-tint": props.tint,
        "letter-spacing": "0.01em",
      }}
    >
      {props.text}
    </span>
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
        "box-shadow": "0 0 0 1.5px var(--color-pill-surface), 0 0 8px rgba(167, 139, 250, 0.55)",
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
