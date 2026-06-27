/**
 * Outline heart — reads as "support / donate" without the commercial
 * loudness of a money or cart glyph. Used by the tray popover's
 * permanent "Support Vox" row and the milestone donation nudge so both
 * surfaces share one affordance. Distinct from `SparkleGlyph` (which is
 * reserved for "polish / fresh thing arrived" moments).
 */
export function HeartGlyph(props: {
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
      <path d="M12 20s-7-4.35-9.5-8.5C1 8.5 2.5 5.5 5.5 5.5c2 0 3.2 1.2 4.5 3 1.3-1.8 2.5-3 4.5-3 3 0 4.5 3 3 6-2.5 4.15-9.5 8.5-9.5 8.5z" />
    </svg>
  );
}
