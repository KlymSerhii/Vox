export type Appearance = "light" | "dark";

const STORAGE_KEY = "vox-theme";

export function applyTheme(theme: Appearance): void {
  const t: Appearance = theme === "light" ? "light" : "dark";
  document.documentElement.dataset.theme = t;
  // Native form controls (`<select>`, scrollbars, `::-webkit-…`)
  // inherit from `color-scheme`, so the early-paint script's
  // `data-theme` attr alone leaves them painting with the OS default
  // hue. Mirroring the theme onto `colorScheme` keeps native widgets
  // matched to the rest of the UI.
  document.documentElement.style.colorScheme = t === "light" ? "light" : "dark";
  try {
    localStorage.setItem(STORAGE_KEY, t);
  } catch {
    // Storage may be unavailable (private mode, blocked) — the
    // `data-theme` attr is the source of truth at runtime, so failing
    // to persist is non-fatal.
  }
}
