/**
 * Shared hotkey-spec → display formatters.
 *
 * Two presentations are exposed:
 *  - `formatHotkeyTokens(spec)` returns the chord split into one token
 *    per keycap (e.g. `[⌘ Cmd, ⌥ Opt, .]`). Use this when rendering
 *    keycap badges so each modifier shows both its symbol and its
 *    short macOS word ("Cmd", "Opt", "Shift", "Ctrl").
 *  - `formatHotkey(spec)` joins the tokens into a single plain string
 *    (e.g. `"Cmd + Opt + ."`). Use this in text contexts (pill toasts,
 *    fallback notes, diagnostic dumps) where rich keycap markup
 *    can't render.
 *
 * When `Shift` combines with a key whose shifted glyph is more
 * legible than `Shift+<key>` (e.g. `Slash` → `?`), the Shift modifier
 * is dropped and the shifted key glyph is rendered alone. This
 * matches macOS menu conventions — `Cmd+Shift+/` shows as `⌘?`, not
 * `⌘⇧/`.
 */

export interface HotkeyToken {
  /** "modifier" for ⌘/⌥/⇧/⌃, "key" for the trailing letter or symbol. */
  kind: "modifier" | "key";
  /** Mac glyph: `⌘`, `⌥`, `⇧`, `⌃`, or a single character like `.` or `M`. */
  symbol: string;
  /** Short word for modifiers (`Cmd`, `Opt`, `Shift`, `Ctrl`); empty for keys. */
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

export function formatHotkeyTokens(
  spec: string | null | undefined,
): HotkeyToken[] {
  if (!spec) return [];
  const trimmed = spec.trim();
  if (!trimmed) return [];

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
  return tokens.map((t) => (t.label ? t.label : t.symbol)).join(" + ");
}
