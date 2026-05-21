import { Show } from "solid-js";
import { AlertIcon, CheckIcon, WarningIcon, XIcon } from "./icons";

export function TerminalLine(props: {
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
