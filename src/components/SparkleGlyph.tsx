/**
 * Liquid-style 4-point sparkle — reads as "polish / refresh" without
 * looking like a copy or refresh-arrow glyph (both of which already
 * appear elsewhere in the popover). Shared by the tray popover's
 * re-polish trigger and the onboarding tips step's Repolish illustration
 * so the two surfaces show the same affordance.
 */
export function SparkleGlyph(props: {
  width?: number | string;
  height?: number | string;
  class?: string;
}) {
  return (
    <svg
      width={props.width ?? 14}
      height={props.height ?? 14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      class={props.class}
      aria-hidden="true"
    >
      <path d="M12 3v4M12 17v4M3 12h4M17 12h4" />
      <path d="M12 8c.5 2 1.5 3 4 3.5C13.5 12 12.5 13 12 16c-.5-3-1.5-4-4-4.5C10.5 11 11.5 10 12 8z" />
    </svg>
  );
}
