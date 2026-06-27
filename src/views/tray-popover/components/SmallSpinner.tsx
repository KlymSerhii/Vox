export function SmallSpinner() {
  // Sized to match the 14×14 svgs already used by the row actions so
  // the spinner slots in without shifting layout.
  return (
    <svg
      class="animate-spin"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-accent)"
      stroke-width="2.5"
      stroke-linecap="round"
      aria-hidden="true"
    >
      <path d="M12 3a9 9 0 1 0 9 9" />
    </svg>
  );
}
