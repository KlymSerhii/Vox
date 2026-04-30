import { createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type PillState =
  | "listening"
  | "transcribing"
  | "polishing"
  | "done"
  | "error"
  | "canceled";

interface PillStatePayload {
  state: PillState;
  message?: string;
  /** Soft-fail flag on `Done` — clipboard succeeded but a step
   *  (typically LLM polish) was substituted. Drives the amber
   *  warning treatment so the user notices their cleanup was
   *  skipped instead of mistaking it for a successful polish. */
  warning?: boolean;
}

interface AudioLevelPayload {
  level: number;
}

const BARS = 32;

export default function PillView() {
  const [state, setState] = createSignal<PillState>("listening");
  const [message, setMessage] = createSignal<string | null>(null);
  const [warning, setWarning] = createSignal(false);
  const [levels, setLevels] = createSignal<number[]>(new Array(BARS).fill(0));

  let unlisten: UnlistenFn[] = [];

  onMount(async () => {
    document.body.classList.add("translucent");

    unlisten.push(
      await listen<PillStatePayload>("pill:state", (e) => {
        setState(e.payload.state);
        setMessage(e.payload.message ?? null);
        setWarning(e.payload.warning ?? false);
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
      }),
    );
  });

  onCleanup(() => {
    for (const u of unlisten) u();
  });

  const isDone = () => state() === "done";
  const isDoneWarning = () => isDone() && warning();
  const isDoneSuccess = () => isDone() && !warning();
  const isProcessing = () =>
    state() === "transcribing" || state() === "polishing";

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
        class="animate-pill-in pointer-events-none flex h-[26px] w-[116px] items-center justify-center rounded-full px-1 transition-shadow duration-300"
        style={{
          background: "var(--color-pill-surface)",
          "backdrop-filter": "var(--glass-blur)",
          "-webkit-backdrop-filter": "var(--glass-blur)",
          border: "1px solid var(--glass-rim)",
          "box-shadow": isDoneSuccess()
            ? "inset 0 0 0 1.5px var(--color-success), 0 0 18px rgba(74, 222, 128, 0.42), var(--glass-shadow-pill)"
            : isDoneWarning()
              ? "inset 0 0 0 1.5px var(--color-warning), 0 0 18px rgba(251, 191, 36, 0.38), var(--glass-shadow-pill)"
              : "var(--glass-shadow-pill)",
        }}
        role="status"
        aria-live="polite"
      >
        <Show when={state() === "listening"}>
          <Waveform levels={levels()} color={meterColor()} />
        </Show>

        <Show when={isProcessing()}>
          <ShimmerWord
            text={state() === "transcribing" ? "Transcribing" : "Polishing"}
            tint={meterColor()}
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
