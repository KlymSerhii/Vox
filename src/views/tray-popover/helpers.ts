import { type Stage } from "@/lib/ipc";

export function stageLabel(stage: Stage): string {
  switch (stage) {
    case "listening":
      return "Listening…";
    case "transcribing":
      return "Transcribing…";
    case "polishing":
      return "Polishing…";
    case "done":
      return "Done";
    case "error":
      return "Error";
    case "canceled":
      return "Canceled";
    default:
      return "…";
  }
}

export function shortRepolishError(message: string): string {
  // The backend returns string errors from `Result<_, String>`. Trim
  // boilerplate (`anyhow` chain, IPC framing) for the dense popover.
  const trimmed = message.replace(/^Error:\s*/i, "").trim();
  if (trimmed.length <= 80) return trimmed;
  return trimmed.slice(0, 77) + "…";
}

export function formatDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds)) return "0s";
  const t = Math.round(totalSeconds);
  if (t < 1) return "0s";
  if (t < 60) return `${t}s`;
  if (t < 3600) {
    const m = Math.floor(t / 60);
    const s = t - m * 60;
    return s === 0 ? `${m}m` : `${m}m ${s}s`;
  }
  const h = Math.floor(t / 3600);
  const m = Math.floor((t - h * 3600) / 60);
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

export function sessionsSubLine(n: number): string {
  if (n === 0) return "No sessions yet";
  if (n === 1) return "1 session";
  return `${n} sessions`;
}

export function ago(tsSeconds: number): string {
  const now = Date.now() / 1000;
  const delta = Math.max(0, Math.floor(now - tsSeconds));
  if (delta < 60) return `${delta}s`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m`;
  if (delta < 86_400) return `${Math.floor(delta / 3600)}h`;
  return `${Math.floor(delta / 86_400)}d`;
}
