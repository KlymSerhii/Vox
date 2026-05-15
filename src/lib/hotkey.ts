/**
 * Shared hotkey-spec → display formatters + parsers.
 *
 * Supported wire formats:
 *   - Legacy single-key + modifier: `Cmd+Alt+Period`, `Alt+Shift+M`,
 *     `F13`, `Escape`, `AltLeft`.
 *   - Modifier-only chord: `Mods:Cmd+Alt`. Routed through
 *     `mac_chord.rs`; needs Accessibility on macOS.
 *   - Double-tap modifier: `Tap2:Alt`. Tap the modifier twice within
 *     ~400 ms with an empty in-between state; needs Accessibility.
 *
 * Two presentations are exposed:
 *  - `formatHotkeyTokens(spec)` returns the chord split into one token
 *    per keycap (e.g. `[⌘ Cmd, ⌥ Opt, .]`). For `Tap2:` it appends a
 *    "×2" suffix token to the lone modifier; for `Mods:` it returns
 *    only the modifier tokens, no trailing key.
 *  - `formatHotkey(spec)` joins the tokens into a single plain string
 *    (e.g. `"Cmd + Opt + ."`). For `Tap2:Alt` it returns `"Opt ×2"`.
 *
 * When `Shift` combines with a key whose shifted glyph is more
 * legible than `Shift+<key>` (e.g. `Slash` → `?`), the Shift modifier
 * is dropped and the shifted key glyph is rendered alone. This
 * matches macOS menu conventions — `Cmd+Shift+/` shows as `⌘?`, not
 * `⌘⇧/`.
 */

export interface HotkeyToken {
  /** "modifier" for ⌘/⌥/⇧/⌃, "key" for the trailing letter or symbol,
   *  "tap2" for the `×2` suffix that follows a double-tap modifier. */
  kind: "modifier" | "key" | "tap2";
  /** Mac glyph: `⌘`, `⌥`, `⇧`, `⌃`, or a single character like `.` or `M`.
   *  Empty string for `tap2` tokens. */
  symbol: string;
  /** Short word for modifiers (`Cmd`, `Opt`, `Shift`, `Ctrl`); empty for
   *  keys. `×2` for `tap2` tokens. */
  label: string;
}

const MODIFIER_SYMBOLS: Record<string, string> = {
  Alt: "⌥",
  Option: "⌥",
  Shift: "⇧",
  Cmd: "⌘",
  Command: "⌘",
  Meta: "⌘",
  Super: "⌘",
  Ctrl: "⌃",
  Control: "⌃",
  CommandOrControl: "⌘",
};

const MODIFIER_LABELS: Record<string, string> = {
  "⌘": "Cmd",
  "⌥": "Opt",
  "⇧": "Shift",
  "⌃": "Ctrl",
};

const KEY_UNSHIFTED: Record<string, string> = {
  Slash: "/",
  Semicolon: ";",
  Backslash: "\\",
  Comma: ",",
  Period: ".",
  Quote: "'",
};

const KEY_SHIFTED: Record<string, string> = {
  Slash: "?",
  Semicolon: ":",
  Backslash: "|",
  Comma: "<",
  Period: ">",
  Quote: "\"",
};

/** Recognise the modifier-only chord shape `Mods:Cmd+Alt`. */
export function isModsSpec(spec: string | null | undefined): boolean {
  return !!spec && spec.trim().startsWith("Mods:");
}

/** Recognise the double-tap shape `Tap2:Alt`. */
export function isTap2Spec(spec: string | null | undefined): boolean {
  return !!spec && spec.trim().startsWith("Tap2:");
}

/** Strip the prefix from a `Mods:` or `Tap2:` spec, returning the body. */
function specBody(spec: string): string {
  const trimmed = spec.trim();
  if (trimmed.startsWith("Mods:")) return trimmed.slice(5).trim();
  if (trimmed.startsWith("Tap2:")) return trimmed.slice(5).trim();
  return trimmed;
}

function modifierTokensFrom(parts: string[]): HotkeyToken[] {
  return parts.map<HotkeyToken>((t) => {
    const symbol = MODIFIER_SYMBOLS[t];
    if (symbol) {
      return { kind: "modifier", symbol, label: MODIFIER_LABELS[symbol] ?? "" };
    }
    return { kind: "modifier", symbol: t, label: "" };
  });
}

export function formatHotkeyTokens(
  spec: string | null | undefined,
): HotkeyToken[] {
  if (!spec) return [];
  const trimmed = spec.trim();
  if (!trimmed) return [];

  if (isTap2Spec(trimmed)) {
    const body = specBody(trimmed);
    if (!body) return [];
    const mods = modifierTokensFrom(
      body.split("+").map((t) => t.trim()).filter(Boolean),
    );
    if (mods.length === 0) return [];
    return [...mods, { kind: "tap2", symbol: "", label: "×2" }];
  }

  if (isModsSpec(trimmed)) {
    const body = specBody(trimmed);
    if (!body) return [];
    return modifierTokensFrom(
      body.split("+").map((t) => t.trim()).filter(Boolean),
    );
  }

  // Right/Left Alt is the only single-modifier spec we display — it
  // arrives without a trailing key.
  if (trimmed === "AltRight" || trimmed === "AltLeft") {
    return [{ kind: "modifier", symbol: "⌥", label: "Opt" }];
  }

  const tokens = trimmed.split("+").map((t) => t.trim()).filter(Boolean);
  if (tokens.length === 0) return [];

  const last = tokens[tokens.length - 1];
  const hasShift = tokens.includes("Shift");
  const collapseShift = hasShift && Object.prototype.hasOwnProperty.call(KEY_SHIFTED, last);

  const mods = tokens
    .slice(0, -1)
    .filter((t) => !collapseShift || t !== "Shift")
    .map<HotkeyToken>((t) => {
      const symbol = MODIFIER_SYMBOLS[t];
      if (symbol) {
        return { kind: "modifier", symbol, label: MODIFIER_LABELS[symbol] ?? "" };
      }
      // Unknown token — render verbatim so users see *something*
      // useful for hand-rolled specs.
      return { kind: "modifier", symbol: t, label: "" };
    });

  const tail: HotkeyToken = collapseShift
    ? { kind: "key", symbol: KEY_SHIFTED[last], label: "" }
    : {
        kind: "key",
        symbol: KEY_UNSHIFTED[last] ?? MODIFIER_SYMBOLS[last] ?? last,
        label: MODIFIER_SYMBOLS[last] ? (MODIFIER_LABELS[MODIFIER_SYMBOLS[last]] ?? "") : "",
      };

  // If the trailing token is itself a modifier (the AltLeft/AltRight
  // case is handled above, but a user-rolled "Shift" spec could land
  // here), promote it to a modifier-style token so the label renders.
  if (MODIFIER_SYMBOLS[last]) {
    tail.kind = "modifier";
  }

  return [...mods, tail];
}

/**
 * Plain-text rendering: each token shown as its short word when
 * available, falling back to the glyph. Tokens are separated by
 * ` + ` so it reads cleanly in fallback notes.
 */
export function formatHotkey(spec: string | null | undefined): string {
  const tokens = formatHotkeyTokens(spec);
  if (tokens.length === 0) return "";
  if (isTap2Spec(spec)) {
    // "Opt ×2" reads more naturally than "Opt + ×2".
    const out: string[] = [];
    for (const t of tokens) {
      if (t.kind === "tap2") {
        if (out.length > 0) out[out.length - 1] = `${out[out.length - 1]} ×2`;
      } else {
        out.push(t.label ? t.label : t.symbol);
      }
    }
    return out.join(" + ");
  }
  return tokens.map((t) => (t.label ? t.label : t.symbol)).join(" + ");
}

export type HotkeyRowKind = "ptt" | "cancel" | "cycle";

export type ParsedSpec = {
  /** "single" for legacy `Cmd+Alt+Period`, "mods" for `Mods:Cmd+Alt`,
   *  "tap2" for `Tap2:Alt`, or "empty" when the spec is blank. */
  shape: "single" | "mods" | "tap2" | "empty";
  cmd: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  /** Canonicalised key token (`M`, `Period`, `F13`, `Escape`, …) or
   *  `""` when no non-modifier token was found. */
  key: string;
};

function normaliseKey(raw: string): string {
  if (/^Key[A-Z]$/.test(raw)) return raw.slice(3);
  if (/^Digit[0-9]$/.test(raw)) return raw.slice(5);
  if (raw.toLowerCase() === "esc") return "Escape";
  return raw;
}

function bucketModifier(token: string): keyof Omit<ParsedSpec, "shape" | "key"> | null {
  const u = token.toUpperCase();
  if (
    u === "COMMAND" ||
    u === "CMD" ||
    u === "SUPER" ||
    u === "META" ||
    u === "COMMANDORCONTROL" ||
    u === "CMDORCTRL" ||
    u === "CMDORCONTROL" ||
    u === "COMMANDORCTRL"
  ) {
    return "cmd";
  }
  if (u === "CONTROL" || u === "CTRL") return "ctrl";
  if (u === "OPTION" || u === "ALT") return "alt";
  if (u === "SHIFT") return "shift";
  return null;
}

/**
 * Parse a Tauri-style or `Mods:`/`Tap2:`-prefixed spec into its parts.
 * Used by both `validateHotkeySpec` and `canonicalSpec` so the two
 * stay in lockstep — drift between them was a v1 staff-review item.
 */
export function parseSpec(spec: string | null | undefined): ParsedSpec {
  const out: ParsedSpec = {
    shape: "empty",
    cmd: false,
    ctrl: false,
    alt: false,
    shift: false,
    key: "",
  };
  if (!spec) return out;
  const trimmed = spec.trim();
  if (!trimmed) return out;

  if (isTap2Spec(trimmed)) {
    out.shape = "tap2";
    const body = specBody(trimmed);
    for (const tok of body.split("+").map((t) => t.trim()).filter(Boolean)) {
      const bucket = bucketModifier(tok);
      if (bucket) (out as Record<string, unknown>)[bucket] = true;
    }
    return out;
  }
  if (isModsSpec(trimmed)) {
    out.shape = "mods";
    const body = specBody(trimmed);
    for (const tok of body.split("+").map((t) => t.trim()).filter(Boolean)) {
      const bucket = bucketModifier(tok);
      if (bucket) (out as Record<string, unknown>)[bucket] = true;
    }
    return out;
  }

  out.shape = "single";
  for (const p of trimmed.split("+").map((t) => t.trim()).filter(Boolean)) {
    const bucket = bucketModifier(p);
    if (bucket) {
      (out as Record<string, unknown>)[bucket] = true;
    } else {
      out.key = normaliseKey(p);
    }
  }
  return out;
}

/** Canonical wire form, used for cross-row conflict comparison. */
export function canonicalSpec(spec: string | null | undefined): string {
  const p = parseSpec(spec);
  if (p.shape === "empty") return "";
  const mods: string[] = [];
  if (p.cmd) mods.push("Cmd");
  if (p.ctrl) mods.push("Ctrl");
  if (p.alt) mods.push("Alt");
  if (p.shift) mods.push("Shift");
  if (p.shape === "mods") return `Mods:${mods.join("+")}`;
  if (p.shape === "tap2") return `Tap2:${mods.join("+")}`;
  const parts = [...mods];
  if (p.key) parts.push(p.key);
  return parts.join("+");
}

export function specsEqual(
  a: string | null | undefined,
  b: string | null | undefined,
): boolean {
  return canonicalSpec(a) === canonicalSpec(b);
}

export type ValidationResult =
  | { kind: "ok" }
  | { kind: "ok-needs-ax" }
  | { kind: "invalid"; reason: string };

/**
 * Validate a captured spec against the per-row rules.
 *
 *   - PTT rows accept `single`, `Mods:`, and `Tap2:` shapes.
 *   - Cancel / Cycle rows only accept `single` (they bind via the
 *     shortcut plugin, not CGEventTap).
 *   - `Tap2:` requires exactly one modifier.
 *   - `Mods:` requires at least one modifier.
 *   - When the result is `Mods:` or `Tap2:` and `ax` is `false`, the
 *     validator returns `ok-needs-ax` so the UI can disable Save and
 *     surface the upsell. With `ax=true` it returns `ok`.
 */
export function validateHotkeySpec(
  spec: string,
  row: HotkeyRowKind,
  ax: boolean,
): ValidationResult {
  if (!spec) return { kind: "invalid", reason: "Pick a shortcut, not nothing." };
  const p = parseSpec(spec);

  if (p.shape === "empty") {
    return { kind: "invalid", reason: "Pick a shortcut, not nothing." };
  }

  if (p.shape === "tap2") {
    if (row !== "ptt") {
      return {
        kind: "invalid",
        reason: "Double-tap shortcuts are push-to-talk only.",
      };
    }
    const count = (p.cmd ? 1 : 0) + (p.ctrl ? 1 : 0) + (p.alt ? 1 : 0) + (p.shift ? 1 : 0);
    if (count !== 1) {
      return {
        kind: "invalid",
        reason: "Double-tap binds a single modifier — pick one.",
      };
    }
    return ax ? { kind: "ok" } : { kind: "ok-needs-ax" };
  }

  if (p.shape === "mods") {
    if (row !== "ptt") {
      return {
        kind: "invalid",
        reason: "Modifier-only chords are push-to-talk only.",
      };
    }
    if (!p.cmd && !p.ctrl && !p.alt && !p.shift) {
      return {
        kind: "invalid",
        reason: "Modifier-only chord needs at least one modifier.",
      };
    }
    if (
      (p.cmd ? 1 : 0) + (p.ctrl ? 1 : 0) + (p.alt ? 1 : 0) + (p.shift ? 1 : 0) === 1 &&
      p.shift
    ) {
      return {
        kind: "invalid",
        reason: "Needs a regular key, not Shift on its own.",
      };
    }
    return ax ? { kind: "ok" } : { kind: "ok-needs-ax" };
  }

  if (!p.key) {
    return {
      kind: "invalid",
      reason: "Add a regular key, not modifiers on their own.",
    };
  }
  const modifierCount =
    (p.cmd ? 1 : 0) + (p.ctrl ? 1 : 0) + (p.alt ? 1 : 0) + (p.shift ? 1 : 0);
  const isFunctionKey = /^F([1-9]|1[0-9]|20)$/.test(p.key);
  const isEscape = p.key === "Escape";

  if (isEscape && row !== "cancel") {
    return {
      kind: "invalid",
      reason: "Esc is reserved for cancelling capture. Pick another.",
    };
  }
  if (isFunctionKey) return { kind: "ok" };
  if (isEscape) return { kind: "ok" };
  if (modifierCount === 0) {
    return {
      kind: "invalid",
      reason: "Add Cmd, Opt, Ctrl, or Shift to a regular key.",
    };
  }
  if (modifierCount === 1 && p.shift) {
    return {
      kind: "invalid",
      reason: "Needs a regular key, not Shift on its own.",
    };
  }
  return { kind: "ok" };
}

/**
 * Build a `Mods:`-shape spec out of the currently-held modifiers, or
 * `null` if no modifiers are held. Used by the live capture handler.
 */
export function modsSpecFromFlags(flags: {
  cmd: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
}): string | null {
  const parts: string[] = [];
  if (flags.cmd) parts.push("Cmd");
  if (flags.ctrl) parts.push("Ctrl");
  if (flags.alt) parts.push("Alt");
  if (flags.shift) parts.push("Shift");
  if (parts.length === 0) return null;
  return `Mods:${parts.join("+")}`;
}

/** Build a `Tap2:<Modifier>` spec for a single tracked modifier. */
export function tap2SpecFromModifier(mod: "Cmd" | "Ctrl" | "Alt" | "Shift"): string {
  return `Tap2:${mod}`;
}
