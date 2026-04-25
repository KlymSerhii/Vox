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

export default function PillView() {
  const [state, setState] = createSignal<PillState>("listening");
  const [message, setMessage] = createSignal<string | null>(null);
  const [levels, setLevels] = createSignal<number[]>(new Array(BARS).fill(0));
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
      }),
    );
    unlisten.push(
      await listen<AudioLevelPayload>("pill:audio_level", (e) => {
        // Shift bars left and push the new sample on the right.
        setLevels((prev) => {
          const next = prev.slice(1);
          next.push(clamp(e.payload.level));
          return next;
        });
      }),
    );
    unlisten.push(
      await listen("pill:show", () => {
        // Reset bars when a new session starts.
        setLevels(new Array(BARS).fill(0));
        setSuggestion(null);
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

  // Done state should show a brief green flash before the pill auto-
  // hides (Rust calls hide_after with 700ms). We mirror that timing on
  // the visual layer so the inset glow runs for the full visible
  // window. CSS handles the actual transition.
  const isDone = () => state() === "done";

  return (
    <div class="flex h-screen w-screen items-center justify-center bg-transparent">
      <div
        class="animate-pill-in pointer-events-none flex h-14 w-[272px] items-center gap-3 rounded-full px-4 transition-shadow duration-300"
        style={{
          background: "rgba(11, 11, 13, 0.82)",
          "backdrop-filter": "blur(24px) saturate(180%)",
          "-webkit-backdrop-filter": "blur(24px) saturate(180%)",
          "box-shadow": isDone()
            ? "0 8px 32px rgba(0,0,0,0.32), inset 0 0 0 2px var(--color-success), 0 0 24px rgba(74, 222, 128, 0.55)"
            : "0 8px 32px rgba(0,0,0,0.32), inset 0 0 0 1px rgba(255,255,255,0.08)",
        }}
        role="status"
        aria-live="polite"
      >
        <LeftIcon state={state()} />
        <div class="min-w-0 flex-1">
          <Show
            when={state() === "listening"}
            fallback={
              <span class="block truncate text-[13px] font-medium text-[var(--color-ink)]">
                {pillLabel(state(), message())}
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
            "text-[var(--color-success)]": state() === "done",
            "text-[var(--color-danger)]": state() === "error",
            "text-[var(--color-ink-mute)] italic": state() === "canceled",
          }}
        >
          {rightLabel(state())}
        </span>
      </div>
      <Show when={state() === "listening" ? suggestion() : null}>
        {(sg) => <SuggestionHud name={sg().name} icon={sg().icon} />}
      </Show>
    </div>
  );
}

function ModeChip(props: { name: string; icon: string }) {
  // Small read-only chip on the listening pill. Switching is done via
  // the tray popover or the cycle hotkey — never on the pill itself.
  return (
    <span
      class="inline-flex shrink-0 items-center gap-1 rounded-full bg-zinc-800/70 px-2 py-0.5 text-[11px] font-medium text-[var(--color-ink)] ring-1 ring-white/10"
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
  // Floats just to the right of the pill — non-intrusive, never blocks
  // the user's flow. Tells them how to switch with the cycle hotkey.
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
        class="flex items-center gap-1.5 rounded-full bg-zinc-800/80 px-2.5 py-1 text-[11px] text-[var(--color-ink)] ring-1 ring-white/10"
        style={{
          "backdrop-filter": "blur(16px) saturate(180%)",
          "-webkit-backdrop-filter": "blur(16px) saturate(180%)",
        }}
      >
        <span aria-hidden="true">{props.icon}</span>
        <span>Switch to {props.name}?</span>
        <span class="rounded bg-white/8 px-1.5 py-0.5 text-[10px] text-[var(--color-ink-mute)]">
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

function pillLabel(state: PillState, message: string | null): string {
  if (message) return message;
  switch (state) {
    case "transcribing":
      return "Transcribing…";
    case "polishing":
      return "Polishing…";
    case "done":
      return "Pasted";
    case "canceled":
      return "Canceled";
    case "error":
      return "Error";
    default:
      return "";
  }
}

function rightLabel(state: PillState): string {
  switch (state) {
    case "listening":
      return "Listening…";
    case "transcribing":
      return "Transcribing";
    case "polishing":
      return "Polishing";
    case "done":
      return "Done";
    case "canceled":
      return "Canceled";
    case "error":
      return "Error";
  }
}

function LeftIcon(props: { state: PillState }) {
  return (
    <div class="flex h-7 w-7 shrink-0 items-center justify-center">
      <Show when={props.state === "listening"}>
        <MicIcon pulse />
      </Show>
      <Show
        when={
          props.state === "transcribing" || props.state === "polishing"
        }
      >
        <Spinner violet={props.state === "polishing"} />
      </Show>
      <Show when={props.state === "done"}>
        <CheckIcon />
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

function MicIcon(props: { pulse?: boolean }) {
  return (
    <svg
      class={props.pulse ? "animate-pulse-mic" : ""}
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-accent)"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <rect x="9" y="2" width="6" height="12" rx="3" />
      <path d="M5 10a7 7 0 0 0 14 0" />
      <line x1="12" y1="17" x2="12" y2="22" />
    </svg>
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
        const h = Math.max(2, Math.round(level * 24));
        const opacity = 0.4 + level * 0.6;
        return (
          <div
            style={{
              width: "4px",
              height: `${h}px`,
              "border-radius": "2px",
              background: "var(--color-accent)",
              opacity,
            }}
          />
        );
      })}
    </div>
  );
}
