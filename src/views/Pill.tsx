import { createSignal, onCleanup, onMount, Show } from "solid-js";
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

interface ActiveModePayload {
  id: string;
  name: string;
  icon: string;
}

interface ModeSuggestionPayload {
  id: string;
  name: string;
  icon: string;
}

const BARS = 24;
const METER_BARS = 10;

export default function PillView() {
  const [state, setState] = createSignal<PillState>("listening");
  const [message, setMessage] = createSignal<string | null>(null);
  const [warning, setWarning] = createSignal(false);
  const [levels, setLevels] = createSignal<number[]>(new Array(BARS).fill(0));
  const [meter, setMeter] = createSignal<number[]>(
    new Array(METER_BARS).fill(0),
  );
  const [activeMode, setActiveMode] = createSignal<ActiveModePayload | null>(
    null,
  );
  const [suggestion, setSuggestion] = createSignal<ModeSuggestionPayload | null>(
    null,
  );

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
        setMeter((prev) => nextMeter(prev, v));
      }),
    );
    unlisten.push(
      await listen("pill:show", () => {
        setLevels(new Array(BARS).fill(0));
        setMeter(new Array(METER_BARS).fill(0));
        setSuggestion(null);
        setWarning(false);
      }),
    );
    unlisten.push(
      await listen<ActiveModePayload>("pill:active_mode", (e) => {
        setActiveMode(e.payload);
      }),
    );
    unlisten.push(
      await listen<ModeSuggestionPayload | null>("pill:mode_suggestion", (e) => {
        setSuggestion(e.payload ?? null);
      }),
    );
  });

  onCleanup(() => {
    for (const u of unlisten) u();
  });

  const isDone = () => state() === "done";
  const isDoneWarning = () => isDone() && warning();
  const isDoneSuccess = () => isDone() && !warning();

  return (
    <div class="flex h-screen w-screen items-center justify-center bg-transparent">
      <div
        class="animate-pill-in pointer-events-none flex h-14 w-[272px] items-center gap-3 rounded-full px-4 transition-shadow duration-300"
        style={{
          background: "var(--color-surface-glass)",
          "backdrop-filter": "var(--glass-blur)",
          "-webkit-backdrop-filter": "var(--glass-blur)",
          border: "1px solid var(--glass-rim)",
          "box-shadow": isDoneSuccess()
            ? "inset 0 0 0 2px var(--color-success), 0 0 24px rgba(74, 222, 128, 0.55), var(--glass-shadow-stack)"
            : isDoneWarning()
              ? "inset 0 0 0 2px var(--color-warning), 0 0 24px rgba(251, 191, 36, 0.45), var(--glass-shadow-stack)"
              : "var(--glass-shadow-stack)",
        }}
        role="status"
        aria-live="polite"
      >
        <LeftIcon state={state()} meter={meter()} warning={warning()} />
        <div class="min-w-0 flex-1">
          <Show
            when={state() === "listening"}
            fallback={
              <span class="block truncate text-[13px] font-medium text-[var(--color-ink)]">
                {pillLabel(state(), message(), warning())}
              </span>
            }
          >
            <Waveform levels={levels()} />
          </Show>
        </div>
        <Show when={activeMode()}>
          {(m) => <ModeChip name={m().name} icon={m().icon} />}
        </Show>
        <span
          class="shrink-0 text-[12px] font-medium"
          classList={{
            "text-[var(--color-ink-mute)]":
              state() === "listening" ||
              state() === "transcribing" ||
              state() === "polishing",
            "text-[var(--color-success)]": isDoneSuccess(),
            "text-[var(--color-warning)]": isDoneWarning(),
            "text-[var(--color-danger)]": state() === "error",
            "text-[var(--color-ink-mute)] italic": state() === "canceled",
          }}
        >
          {rightLabel(state(), warning())}
        </span>
      </div>
      <Show when={state() === "listening" ? suggestion() : null}>
        {(sg) => <SuggestionHud name={sg().name} icon={sg().icon} />}
      </Show>
    </div>
  );
}

function ModeChip(props: { name: string; icon: string }) {
  return (
    <span
      class="inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium text-[var(--color-ink)]"
      style={{
        background: "var(--color-surface-glass)",
        "backdrop-filter": "blur(18px) saturate(200%)",
        "-webkit-backdrop-filter": "blur(18px) saturate(200%)",
        border: "1px solid var(--glass-rim)",
        "box-shadow": "var(--glass-shadow-stack-sm)",
      }}
      title={`Voice mode: ${props.name}`}
    >
      <span class="leading-none" aria-hidden="true">
        {props.icon}
      </span>
      <span class="max-w-[80px] truncate">{props.name}</span>
    </span>
  );
}

function SuggestionHud(props: { name: string; icon: string }) {
  return (
    <div
      class="pointer-events-none absolute"
      style={{
        bottom: "12px",
        right: "12px",
        display: "flex",
        "align-items": "center",
      }}
    >
      <div
        class="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] text-[var(--color-ink)]"
        style={{
          background: "var(--color-surface-glass)",
          "backdrop-filter": "var(--glass-blur)",
          "-webkit-backdrop-filter": "var(--glass-blur)",
          border: "1px solid var(--glass-rim)",
          "box-shadow": "var(--glass-shadow-stack-sm)",
        }}
      >
        <span aria-hidden="true">{props.icon}</span>
        <span>Switch to {props.name}?</span>
        <span
          class="rounded px-1.5 py-0.5 text-[10px] text-[var(--color-ink-mute)]"
          style={{ background: "var(--color-control)" }}
        >
          ⌥M
        </span>
      </div>
    </div>
  );
}

function clamp(n: number): number {
  if (Number.isNaN(n)) return 0;
  return Math.max(0, Math.min(1, n));
}

// Each bar trails the next by a frame so samples ripple through the
// cluster. Decay 0.85 keeps the leftmost bars alive across 10 bars
// (0.7 damps to nothing before reaching the far edge).
function nextMeter(prev: number[], sample: number): number[] {
  const next = new Array(prev.length).fill(0);
  next[next.length - 1] = sample;
  for (let i = 0; i < next.length - 1; i++) {
    const incoming = prev[i + 1] ?? 0;
    const decayed = (prev[i] ?? 0) * 0.85;
    next[i] = Math.max(decayed, incoming);
  }
  const tailDecayed = (prev[prev.length - 1] ?? 0) * 0.85;
  next[next.length - 1] = Math.max(tailDecayed, sample);
  return next;
}

function pillLabel(
  state: PillState,
  message: string | null,
  _warning: boolean,
): string {
  if (message) return message;
  switch (state) {
    case "transcribing":
      return "Transcribing…";
    case "polishing":
      return "Polishing…";
    case "done":
      return "Copied";
    case "canceled":
      return "Canceled";
    case "error":
      return "Error";
    default:
      return "";
  }
}

function rightLabel(state: PillState, warning: boolean): string {
  switch (state) {
    case "listening":
      return "Listening…";
    case "transcribing":
      return "Transcribing";
    case "polishing":
      return "Polishing";
    case "done":
      return warning ? "Raw copy" : "Done";
    case "canceled":
      return "Canceled";
    case "error":
      return "Error";
  }
}

function LeftIcon(props: {
  state: PillState;
  meter: number[];
  warning: boolean;
}) {
  return (
    <div
      class="flex h-8 shrink-0 items-center justify-center"
      classList={{
        "w-[52px]": props.state === "listening",
        "w-7": props.state !== "listening",
      }}
    >
      <Show when={props.state === "listening"}>
        <MicLevelMeter bars={props.meter} />
      </Show>
      <Show
        when={
          props.state === "transcribing" || props.state === "polishing"
        }
      >
        <Spinner violet={props.state === "polishing"} />
      </Show>
      <Show when={props.state === "done" && !props.warning}>
        <CheckIcon />
      </Show>
      <Show when={props.state === "done" && props.warning}>
        <WarningIcon />
      </Show>
      <Show when={props.state === "error"}>
        <AlertIcon />
      </Show>
      <Show when={props.state === "canceled"}>
        <XIcon />
      </Show>
    </div>
  );
}

function MicLevelMeter(props: { bars: number[] }) {
  const PROFILE = [0.7, 0.82, 0.92, 0.98, 1.0, 1.0, 0.98, 0.92, 0.82, 0.7];
  const MAX_H = 30;
  const MIN_H = 2;
  const GAIN = 1.8;
  return (
    <div
      class="flex h-8 items-center justify-center gap-[2px]"
      aria-hidden="true"
    >
      {props.bars.map((level, i) => {
        const amped = Math.min(1.0, level * GAIN);
        const shaped = amped * (PROFILE[i] ?? 1);
        const h = MIN_H + Math.round(shaped * (MAX_H - MIN_H));
        const active = level > 0.06;
        return (
          <div
            style={{
              width: "3px",
              height: `${h}px`,
              "border-radius": "2px",
              background: "var(--color-meter)",
              "box-shadow": "inset 0 -1px 0 var(--color-meter-shadow)",
              opacity: active ? 0.95 : 0.5,
              transition: "height 120ms ease-out, opacity 200ms ease-out",
            }}
          />
        );
      })}
    </div>
  );
}

function Spinner(props: { violet?: boolean }) {
  const color = props.violet ? "var(--color-violet)" : "var(--color-accent)";
  return (
    <svg
      class="animate-spinner"
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke={color}
      stroke-width="1.75"
      stroke-linecap="round"
      aria-hidden="true"
    >
      <path d="M21 12a9 9 0 1 1-6.2-8.5" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-success)"
      stroke-width="2"
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
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-danger)"
      stroke-width="1.75"
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
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-warning)"
      stroke-width="1.75"
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
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-ink-mute)"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <line x1="6" y1="6" x2="18" y2="18" />
      <line x1="6" y1="18" x2="18" y2="6" />
    </svg>
  );
}

function Waveform(props: { levels: number[] }) {
  return (
    <div
      class="flex h-6 items-center gap-[2px] overflow-hidden"
      aria-hidden="true"
    >
      {props.levels.map((level) => {
        // Gamma 0.7 opens up the bottom of the range — Whisper-grade
        // RMS spends most of its life in [0, 0.3].
        const shaped = Math.pow(level, 0.7);
        const h = Math.max(1, Math.round(shaped * 24));
        const opacity = 0.25 + shaped * 0.75;
        return (
          <div
            style={{
              width: "4px",
              height: `${h}px`,
              "border-radius": "2px",
              background: "var(--color-meter)",
              "box-shadow": "inset 0 -1px 0 var(--color-meter-shadow)",
              opacity,
            }}
          />
        );
      })}
    </div>
  );
}
