import { Show } from "solid-js";
import { AlertIcon, CheckIcon, WarningIcon, XIcon } from "./icons";

export function TerminalLine(props: {
  tone: "success" | "warning" | "error" | "muted";
  label: string;
}) {
  return (
    <div class="flex items-center gap-1.5">
      {/* Icon never shrinks when a long label wraps to a second line. */}
      <span class="flex shrink-0 items-center">
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
      </span>
      <span
        class="text-[11px] font-medium"
        style={{
          color:
            props.tone === "success"
              ? "var(--color-success)"
              : props.tone === "warning"
                ? "var(--color-warning)"
                : props.tone === "error"
                  ? "var(--color-danger)"
                  : "var(--color-ink-mute)",
          // Long messages (e.g. "Microphone unavailable — check System
          // Settings and try again.") flow onto a second line as the
          // pill grows wider, instead of cramming into the compact
          // silhouette. The clamp is a backstop — at the pill's 360 px
          // width cap every shipped message fits within two lines.
          "line-height": "1.3",
          display: "-webkit-box",
          "-webkit-box-orient": "vertical",
          "-webkit-line-clamp": "2",
          overflow: "hidden",
          "overflow-wrap": "break-word",
        }}
      >
        {props.label}
      </span>
    </div>
  );
}
