import { type HotkeyDiagnostic } from "@/lib/ipc";
import { formatHotkey as sharedFormatHotkey } from "@/lib/hotkey";
import { isMac } from "@/lib/platform";
import { type ProgressSample } from "./constants";

/** Render a Tauri global-shortcut spec ("Alt+Shift+V", "F13") as a
 *  short word-style string ("Opt+Shift+V", "F13"). Re-exported from
 *  the shared `lib/hotkey` formatter; kept as `formatHotkey` here so
 *  the rest of this file doesn't have to know the new import path. */
export const formatHotkey = sharedFormatHotkey;

export function shortModelName(name: string): string {
  // Keep the parenthesised quant tag (e.g. "(Q5_K_M)") so the
  // download bar matches the picker's full label. Only strip the
  // trailing em-dash tier suffix ("— Small"), which is duplicated by
  // the row's RAM/size hints elsewhere on the same screen.
  return name.replace(/ —.*$/, "");
}

/// Compute remaining seconds from a rolling sample window. Returns
/// `null` when there isn't enough signal yet — a single sample or a
/// flat span gives a nonsense throughput, and showing "0s remaining"
/// while a multi-GB GGUF still has 4 GB left to fetch would be worse
/// than no ETA. Throughput is the byte delta across the oldest and
/// newest sample in the window divided by their elapsed time, which is
/// effectively a 3–5 s moving average given the 200 ms emit cadence.
export function etaSeconds(
  prog: { downloaded: number; total: number } | undefined,
  samples: ProgressSample[] | undefined,
): number | null {
  if (!prog || prog.total <= 0 || prog.downloaded >= prog.total) return null;
  if (!samples || samples.length < 2) return null;
  const oldest = samples[0];
  const newest = samples[samples.length - 1];
  const elapsedMs = newest.ts - oldest.ts;
  const deltaBytes = newest.bytes - oldest.bytes;
  if (elapsedMs <= 0 || deltaBytes <= 0) return null;
  const bps = (deltaBytes / elapsedMs) * 1000;
  const remaining = prog.total - prog.downloaded;
  const eta = remaining / bps;
  if (!isFinite(eta) || eta < 0) return null;
  return Math.round(eta);
}

/// Format seconds as `12s remaining`, `1m 23s remaining`, or
/// `1h 04m remaining`. Hours suppressed unless > 1h; minutes are
/// zero-padded only in the hours branch where two-digit alignment
/// keeps the eye from wobbling.
export function formatEta(secs: number): string {
  if (secs < 60) return `${secs}s remaining`;
  if (secs < 3600) {
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    return `${m}m ${s.toString().padStart(2, "0")}s remaining`;
  }
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${m.toString().padStart(2, "0")}m remaining`;
}

export function fellBackNote(d: HotkeyDiagnostic | undefined): string | null {
  if (!d || !d.fell_back || !d.bound) return null;
  const cfg = formatHotkey(d.configured) || d.configured;
  const bnd = formatHotkey(d.bound) || d.bound;
  const here = isMac ? "your Mac" : "your PC";
  return `Configured ${cfg} wasn't available on ${here} — Vox is using ${bnd}.`;
}

export function pttFailed(d: HotkeyDiagnostic | undefined): boolean {
  if (!d) return false;
  return !d.bound && d.configured !== "";
}
