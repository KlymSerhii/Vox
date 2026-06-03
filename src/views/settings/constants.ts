import {
  languageDisplayName,
  PARAKEET_SUPPORTED_LANGUAGES,
  type CaseStyle,
  type PunctuationStyle,
} from "@/lib/ipc";
import { type HotkeyRowKind } from "@/lib/hotkey";
import { deviceWord } from "@/lib/platform";

export type Tab =
  | "general"
  | "hotkeys"
  | "models"
  | "modes"
  | "dictionary"
  | "privacy"
  | "about";

export const TABS: { id: Tab; label: string; icon: string; subtitle: string }[] = [
  {
    id: "general",
    label: "General",
    icon: "sliders",
    subtitle: "How Vox starts, listens, and looks.",
  },
  {
    id: "hotkeys",
    label: "Hotkeys",
    icon: "keyboard",
    subtitle: "Shortcuts for dictating, cancelling, and switching modes.",
  },
  {
    id: "models",
    label: "Models",
    icon: "cube",
    subtitle: "Manage the on-device speech and polishing engines.",
  },
  {
    id: "modes",
    label: "Modes",
    icon: "modes",
    subtitle: "Tone, punctuation, and dictionary per situation.",
  },
  {
    id: "dictionary",
    label: "Dictionary",
    icon: "book",
    subtitle: "Names and acronyms Vox should always spell correctly.",
  },
  {
    id: "privacy",
    label: "Privacy",
    icon: "lock",
    subtitle: `What stays on this ${deviceWord} (everything).`,
  },
  {
    id: "about",
    label: "About",
    icon: "info",
    subtitle: "Version, diagnostics, and updates.",
  },
];

// Defaults written by `settings::load` on every launch — kept in sync
// with `src-tauri/src/settings.rs::default_hotkey` etc.
export const DEFAULT_PUSH_TO_TALK = "Cmd+Alt+Period";
export const DEFAULT_CANCEL_HOTKEY = "Escape";
export const DEFAULT_MODE_CYCLE_HOTKEY = "Alt+M";

// Common chords surfaced inside the capture panel for one-click pick.
// Cancel-only specs are filtered out when the row isn't `cancel_hotkey`.
// `pttOnly` flags Mods: / Tap2: shapes that only the push-to-talk row
// can bind (cancel + mode-cycle go through `tauri-plugin-global-shortcut`,
// not CGEventTap).
// `wide` chords render with 3+ keycaps (e.g. Cmd+Shift+M) and don't fit
// inside a single column of the 2-col picker grid — they span both cols
// so the glyph+word labels stay inside the button border.
export const COMMON_CHORDS: {
  spec: string;
  cancelOnly?: boolean;
  pttOnly?: boolean;
  wide?: boolean;
}[] = [
  { spec: "Escape", cancelOnly: true },
  { spec: "F13" },
  { spec: "F14" },
  { spec: "F15" },
  { spec: "Alt+M" },
  { spec: "Alt+Shift+M", wide: true },
  { spec: "Cmd+Shift+M", wide: true },
  { spec: "Mods:Ctrl+Alt", pttOnly: true },
  { spec: "Tap2:Alt", pttOnly: true },
];

export const PUNCTUATION_OPTIONS: { value: PunctuationStyle; label: string }[] = [
  { value: "Aggressive", label: "Aggressive" },
  { value: "Standard", label: "Standard" },
  { value: "Minimal", label: "Minimal" },
  { value: "None", label: "None (strip)" },
];

export const CASE_OPTIONS: { value: CaseStyle; label: string }[] = [
  { value: "Sentence", label: "Sentence" },
  { value: "Title", label: "Title" },
  { value: "Lower", label: "Lowercase" },
  { value: "AsSpoken", label: "As spoken" },
];

// Common languages that aren't in Parakeet's 25-language set. Mirrors
// `NON_PARAKEET_COMMON` in Onboarding.tsx so both surfaces offer the
// same coverage.
export const NON_PARAKEET_COMMON: readonly string[] = [
  "zh", "ja", "ko", "ar", "hi", "tr", "vi", "th",
] as const;

export const LANGUAGE_OPTIONS: { value: string; label: string }[] = [
  { value: "auto", label: "Auto (detect)" },
  ...[
    ...PARAKEET_SUPPORTED_LANGUAGES,
    ...NON_PARAKEET_COMMON,
  ]
    .map((code) => ({ value: code, label: `${languageDisplayName(code)} (${code})` }))
    .sort((a, b) => a.label.localeCompare(b.label)),
];

export type HotkeyKind = HotkeyRowKind;

/// v2 collapses v1's `recording / preview / preview-invalid /
/// preview-conflict` into one continuous `capturing` state carrying the
/// in-progress set + verdict. The user commits with Save, wipes with
/// Clear, or exits with Cancel.
export type Verdict =
  | { tone: "empty" }
  | { tone: "neutral"; message: string }
  | { tone: "ok"; message: string }
  | { tone: "danger"; message: string }
  | { tone: "needs-ax"; message: string }
  | { tone: "conflict"; message: string; with: { kind: HotkeyKind; label: string } };

export type CaptureSnapshot = {
  /// Captured-so-far set, ready to round-trip as a wire spec. Empty
  /// string means "set is empty".
  spec: string;
  verdict: Verdict;
};

export type CaptureMode =
  | { kind: "idle" }
  | { kind: "capturing"; snap: CaptureSnapshot };

export const EMPTY_SNAP: CaptureSnapshot = {
  spec: "",
  verdict: { tone: "empty" },
};

export const EMPTY_HINT = "Press the shortcut you want… Esc cancels.";
export const MODS_OK_HINT = "Hold a base key, or click Save to bind modifiers alone.";
export const TAP2_OK_HINT = "Looks good. Tap twice within 300 ms to trigger.";
export const TAP1_HINT = "Tap Opt again for double-tap, or hold for a chord.";

/// Window inside which a second tap of the same modifier promotes the
/// capture to a `Tap2:` spec.  PM spec §3c: 400 ms; the runtime trigger
/// uses 300 ms in `mac_chord.rs`, but the capture window is more lenient
/// so users don't have to nail the timing on first try.
export const DOUBLE_TAP_WINDOW_MS = 400;
