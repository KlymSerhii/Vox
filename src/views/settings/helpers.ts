import { type DiagnoseHotkeys, type HotkeyDiagnostic } from "@/lib/ipc";
import { isMac } from "@/lib/platform";

export function friendlySttEngineName(engine: string): string {
  if (engine === "parakeet-local") return "Parakeet";
  if (engine === "whisper-local") return "Whisper";
  return "The active engine";
}

export function pttHint(
  handsFree: boolean,
  diag: HotkeyDiagnostic | null,
): string {
  if (handsFree) {
    return "Press once to start recording, press again to stop. (Hands-free is on.)";
  }
  if (diag?.mechanism === "chord") {
    return "Hold both keys to dictate. Release either to stop.";
  }
  return "Hold the key to dictate. Release to stop.";
}

export function formatBytes(b: number) {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(0)} KB`;
  if (b < 1024 * 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MB`;
  return `${(b / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

// One-line, user-facing description per model. DESIGN.md §5.6 — explain
// what *changes* for the user (speed / quality trade-offs), not RAM
// numbers (those already live in the metadata row underneath).
export function modelBlurb(id: string): string | null {
  switch (id) {
    case "parakeet-tdt-0.6b-v3":
      return isMac
        ? "Multilingual (25 languages). Streams transcripts straight onto the Apple Neural Engine."
        : "Multilingual (25 languages). GPU-accelerated via DirectML on your PC.";
    case "whisper-tiny-en-q5_1":
      return "Fastest. English-only. Good for short notes; misses uncommon words.";
    case "whisper-small-en-q5_1":
      return "Fast. English-only. Better with technical terms and names.";
    case "whisper-large-v3-turbo-q5_0":
      return "Multilingual. Sharp accuracy with near-realtime turnaround.";
    case "whisper-large-v3-q5_0":
      return "Multilingual. Best accuracy on long or noisy dictations.";
    case "gemma-4-e2b-q5_k_m":
      return "Snappy cleanup. Punctuation and capitalisation; light filler-word removal.";
    case "gemma-4-e4b-q5_k_m":
      return "Balanced. Reliable filler-word removal and natural sentence flow.";
    case "gemma-4-26b-a4b-q4_k_m":
      return "Slowest. Best at preserving your voice and intent over long dictations.";
    default:
      return null;
  }
}

export function formatDiagnostics(d: DiagnoseHotkeys): string {
  if (d.entries.length === 0) {
    return "No hotkey state available — restart the app.";
  }
  const lines: string[] = [];
  for (const e of d.entries) {
    const cfg = e.configured || "<off>";
    const bnd = e.bound || "<unbound>";
    const mech = e.mechanism ? ` mechanism=${e.mechanism}` : "";
    lines.push(
      `${e.purpose.padEnd(14)} configured=${cfg.padEnd(28)} bound=${bnd}${mech}` +
      (e.fell_back ? "  [fallback]" : ""),
    );
    if (e.error_chain.length > 0) {
      for (const err of e.error_chain) {
        lines.push(`               · ${err}`);
      }
    }
  }
  return lines.join("\n");
}
