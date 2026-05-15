export type PolishChoice = "apple-fm" | "gemma-local";

export type Step = 0 | 1 | 2 | 3 | 4 | 5;

export const STEP_COUNT = 6;
export const LAST_STEP: Step = 5;
export const TIPS_STEP: Step = 4;
export const MODELS_STEP: Step = 3;
export const MIC_STEP: Step = 2;
export const LANGUAGE_STEP: Step = 1;

// Common languages that aren't in Parakeet's 25-language set. Listed
// here so the language picker can offer them as a one-tap "this routes
// to Whisper" path without forcing the user into a free-form input.
export const NON_PARAKEET_COMMON: readonly string[] = [
  "zh", "ja", "ko", "ar", "hi", "tr", "vi", "th",
] as const;

// Sentinel for the secondary `<select>` so the user can opt into the
// Parakeet auto-detect path even after first picking an explicit code.
export const AUTO_VALUE = "__auto__";

/// Per-id rolling history of (timestamp, bytes_downloaded) samples used
/// to derive a smoothed throughput estimate for the ETA. The window is
/// the most recent samples within `ETA_WINDOW_MS`; older entries are
/// dropped so the throughput reflects current speed, not a stale
/// resumed-download baseline.
export type ProgressSample = { ts: number; bytes: number };
export const ETA_WINDOW_MS = 5_000;
export const ETA_MAX_SAMPLES = 12;
