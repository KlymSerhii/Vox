// Shared coordinate math for the TrayPopover dropdowns. The popover
// root has `overflow-hidden`, so each dropdown is rendered with
// `position: fixed` and its top/left/bottom/min-width/max-height are
// computed from the trigger's bounding rect. The four call sites
// (mode picker, period picker, mic picker, re-polish picker) used to
// duplicate this math with subtle variations; this helper consolidates
// it so the dropdown DOM consumes a single shape.

export type DropdownPlacement = "below" | "above" | "auto";

export interface DropdownPositionInput {
  // Trigger's bounding rect, from `triggerRef.getBoundingClientRect()`.
  trigger: DOMRect;
  // Viewport extents, typically `{ width: window.innerWidth, height: window.innerHeight }`.
  viewport: { width: number; height: number };
  // Minimum menu width in pixels. Result's `minWidth` is at least
  // `Math.max(trigger.width, minMenuWidth)`.
  minMenuWidth: number;
  // Optional upper cap on menu width. When set, the result's
  // `minWidth` is `Math.min(maxMenuWidth, Math.max(trigger.width, minMenuWidth))`.
  maxMenuWidth?: number;
  // For "auto" placement, the ideal menu height. Used to pick above
  // vs below: stay below if there's room for `desiredHeight`, else
  // pick whichever side has more space.
  desiredHeight?: number;
  // Placement preference. "below" forces below (no flip). "above"
  // forces above. "auto" picks based on `desiredHeight` and available
  // space. Default: "below".
  preferred?: DropdownPlacement;
  // Horizontal anchor. "start" pins to trigger.left, "end" pins to
  // trigger.right (i.e. right-aligned). Default: "start".
  align?: "start" | "end";
  // Pixels between trigger and menu. Default: 4.
  gap?: number;
  // Viewport margin in pixels, used for left/right clamping and
  // top/bottom max-height calculations. Default: 8.
  margin?: number;
}

export interface DropdownPosition {
  placement: "above" | "below";
  // Only one of `top` / `bottom` is set, depending on `placement`.
  // The other field carries 0 so the shape is uniform across all
  // call sites (the dropdown picks the side via `placement`).
  top: number;
  bottom: number;
  left: number;
  minWidth: number;
  // Cap on menu height so the list scrolls instead of clipping past
  // the window edge. Always populated (= space on the chosen side
  // minus the margin).
  maxHeight: number;
}

export function computeDropdownPosition(
  input: DropdownPositionInput,
): DropdownPosition {
  const {
    trigger,
    viewport,
    minMenuWidth,
    maxMenuWidth,
    desiredHeight = 200,
    preferred = "below",
    align = "start",
    gap = 4,
    margin = 8,
  } = input;

  const widthFloor = Math.max(trigger.width, minMenuWidth);
  const minWidth =
    typeof maxMenuWidth === "number"
      ? Math.min(maxMenuWidth, widthFloor)
      : widthFloor;

  const left =
    align === "end"
      ? Math.max(margin, trigger.right - minWidth)
      : Math.max(margin, trigger.left);

  const spaceBelow = viewport.height - trigger.bottom - margin;
  const spaceAbove = trigger.top - margin;

  let placement: "above" | "below";
  if (preferred === "below") {
    placement = "below";
  } else if (preferred === "above") {
    placement = "above";
  } else {
    // "auto": stay below if it fits, otherwise pick the larger side.
    placement =
      spaceBelow >= desiredHeight || spaceBelow >= spaceAbove
        ? "below"
        : "above";
  }

  if (placement === "below") {
    return {
      placement: "below",
      top: trigger.bottom + gap,
      bottom: 0,
      left,
      minWidth,
      maxHeight: Math.max(0, spaceBelow),
    };
  }

  return {
    placement: "above",
    top: 0,
    bottom: viewport.height - trigger.top + gap,
    left,
    minWidth,
    maxHeight: Math.max(0, spaceAbove),
  };
}
