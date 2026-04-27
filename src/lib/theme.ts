export type Appearance = "light" | "dark";

const STORAGE_KEY = "vox-theme";

export function applyTheme(theme: Appearance): void {
  const t: Appearance = theme === "light" ? "light" : "dark";
  document.documentElement.dataset.theme = t;
  try {
    localStorage.setItem(STORAGE_KEY, t);
  } catch {
    // Storage may be unavailable (private mode, blocked) — the
    // `data-theme` attr is the source of truth at runtime, so failing
    // to persist is non-fatal.
  }
}
