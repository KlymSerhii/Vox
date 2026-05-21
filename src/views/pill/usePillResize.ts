import { createSignal, onCleanup } from "solid-js";
import { pillResize } from "@/lib/ipc";
import {
  FLOOR_GRID,
  FLOOR_GROW_TRIGGER,
  FLOOR_SHRINK_SUSTAIN_MS,
  FLOOR_SHRINK_TRIGGER,
  H_MAX,
  H_MIN,
  W_MAX,
  W_MIN,
  WINDOW_PADDING_H,
  WINDOW_PADDING_W,
} from "./constants";

/// Bridges the rendered DOM up to the OS window. When the streaming
/// text grows wider or wraps to N lines the inner pill div's measured
/// size grows; this hook pushes that to Rust via `pill_resize` so the
/// window chrome matches. It also drives the dynamic width-floor lerp
/// that couples width to height so a tall, narrow pill never happens.
///
/// The hook owns both element refs — wire them in JSX via the returned
/// `setWrapperRef` / `setPillRef` callbacks. `wrapperRef` is the outer
/// flex-column container (pill body + optional mode chip); the
/// ResizeObserver watches it so pill_resize accounts for the chip's
/// extra height. `pillRef` targets the inner pill div for the width
/// floor measurement.
export function usePillResize() {
  let wrapperRef: HTMLDivElement | undefined;
  let pillRef: HTMLDivElement | undefined;
  let resizeObserver: ResizeObserver | null = null;
  let resizeRaf: number | null = null;
  // null sentinel = "no size reported yet" so the very first observer
  // emit always wins the change check. Replaces the previous magic
  // 144/36 placeholder, which leaked the compact-pill dimensions into
  // the dedup logic.
  let lastReportedWidth: number | null = null;
  let lastReportedHeight: number | null = null;

  // Signal so the floor lives inside the same reactive style object
  // SolidJS rebuilds on every state change — writing to `pillRef.style.minWidth`
  // imperatively would be clobbered the next time any other style key
  // (e.g. box-shadow flipping on Done) flagged the parent for re-render.
  // 0 is the sentinel "not yet applied" — the parent style attribute's
  // own `min-width` rule wins until the observer raises this above 0.
  const [dynamicMinWidth, setDynamicMinWidth] = createSignal(0);
  let lastAppliedFloor = 0;
  let pendingShrinkFloor: number | null = null;
  let pendingShrinkSince = 0;

  function computeWidthFloor(measuredH: number): number {
    const rawT = (measuredH - H_MIN) / (H_MAX - H_MIN);
    const t = Math.max(0, Math.min(1, rawT));
    // Ease-out: 1 - (1 - t)^2 — fast initial widening so even slight
    // overflow past H_MIN (≈single line) starts opening the box.
    const tEased = 1 - (1 - t) * (1 - t);
    const floor = W_MIN + (W_MAX - W_MIN) * tEased;
    return Math.round(floor / FLOOR_GRID) * FLOOR_GRID;
  }

  function applyDynamicWidthFloor(measuredH: number) {
    if (!pillRef) return;
    const candidate = computeWidthFloor(measuredH);
    const delta = candidate - lastAppliedFloor;
    if (delta === 0) return;
    if (delta >= FLOOR_GROW_TRIGGER) {
      // Grow path commits immediately — content already overflowed,
      // any wait would visibly lag the wrap.
      pendingShrinkFloor = null;
      lastAppliedFloor = candidate;
      setDynamicMinWidth(candidate);
      return;
    }
    if (delta > 0) {
      // Within the dead-band — ignore so we don't churn IPC.
      pendingShrinkFloor = null;
      return;
    }
    // delta < 0: candidate floor is smaller than what we've applied.
    // Only commit shrinks that exceed the wider band AND have been
    // sustained for FLOOR_SHRINK_SUSTAIN_MS — protects against a
    // momentary content height dip yanking the floor down only to
    // immediately bounce back.
    if (-delta < FLOOR_SHRINK_TRIGGER) {
      pendingShrinkFloor = null;
      return;
    }
    const now = performance.now();
    if (pendingShrinkFloor !== candidate) {
      pendingShrinkFloor = candidate;
      pendingShrinkSince = now;
      return;
    }
    if (now - pendingShrinkSince < FLOOR_SHRINK_SUSTAIN_MS) {
      return;
    }
    lastAppliedFloor = candidate;
    pendingShrinkFloor = null;
    setDynamicMinWidth(candidate);
  }

  function resetDynamicWidthFloor() {
    lastAppliedFloor = 0;
    pendingShrinkFloor = null;
    pendingShrinkSince = 0;
    setDynamicMinWidth(0);
  }

  function reportPillSize(contentW: number, contentH: number) {
    // Lower bound matches the Rust WIDE_WIDTH_MIN_DYNAMIC (W_MIN px).
    // Anything below would be clamped up by Rust anyway and just
    // produce an extra IPC round-trip.
    const targetW = Math.max(W_MIN, Math.ceil(contentW + WINDOW_PADDING_W));
    const targetH = Math.max(36, Math.ceil(contentH + WINDOW_PADDING_H));
    // Round to 4 px to soak up sub-pixel jitter from the browser's
    // layout pass — without this the IPC fires on every keystroke.
    const roundedW = Math.ceil(targetW / 4) * 4;
    const roundedH = Math.ceil(targetH / 4) * 4;
    const widthChanged = roundedW !== lastReportedWidth;
    const heightChanged = roundedH !== lastReportedHeight;
    if (!widthChanged && !heightChanged) return;
    lastReportedWidth = roundedW;
    lastReportedHeight = roundedH;
    void pillResize({
      width: widthChanged ? roundedW : undefined,
      height: heightChanged ? roundedH : undefined,
    }).catch(() => {
      // Ignore — IPC failure here just means the window stays the
      // size it was. Worst case the text overflows.
    });
  }

  function setupResizeObserver() {
    if (resizeObserver || !wrapperRef) return;
    resizeObserver = new ResizeObserver((entries) => {
      // Coalesce bursts via rAF so a flurry of streaming chunks only
      // produces one pill_resize per frame.
      if (resizeRaf !== null) cancelAnimationFrame(resizeRaf);
      resizeRaf = requestAnimationFrame(() => {
        resizeRaf = null;
        const last = entries[entries.length - 1];
        if (!last) return;
        // Update the dynamic width floor BEFORE reporting size up so
        // the very next layout pass — which the floor change kicks
        // off — gets reported instead of the pre-floor measurement.
        // Use the inner pill height (not wrapper height) so the chip's
        // extra ~29 px (gap + chip row) doesn't skew the floor lerp.
        const pillHeight = pillRef
          ? pillRef.getBoundingClientRect().height
          : last.contentRect.height;
        applyDynamicWidthFloor(pillHeight);
        // Report the LARGER of the measured content width and the floor
        // we just applied. When a line wrap bumps the height, the floor
        // lerp widens `min-width` — but that widening only materialises
        // on the NEXT layout pass, so `contentRect.width` here still
        // holds the pre-widen value. Sizing the OS window to that stale
        // (narrower) width leaves the pill div rendering wider than its
        // window for a frame or two → the text clips at the sides until
        // the next observer tick catches up. Feeding the floor in now
        // closes that gap so the window never trails a floor jump.
        // reportPillSize still gets the wrapper's full height so the
        // OS window accounts for the chip when it's visible.
        const effectiveWidth = Math.max(
          last.contentRect.width,
          dynamicMinWidth(),
        );
        reportPillSize(effectiveWidth, last.contentRect.height);
      });
    });
    resizeObserver.observe(wrapperRef);
  }

  function teardownResizeObserver() {
    if (resizeObserver) {
      resizeObserver.disconnect();
      resizeObserver = null;
    }
    if (resizeRaf !== null) {
      cancelAnimationFrame(resizeRaf);
      resizeRaf = null;
    }
    // Reset cached dimensions to the null sentinel so the next
    // streaming session reports fresh measurements rather than
    // skipping due to "no change".
    lastReportedWidth = null;
    lastReportedHeight = null;
    resetDynamicWidthFloor();
  }

  onCleanup(teardownResizeObserver);

  return {
    setWrapperRef: (el: HTMLDivElement) => (wrapperRef = el),
    setPillRef: (el: HTMLDivElement) => (pillRef = el),
    dynamicMinWidth,
    setupResizeObserver,
    teardownResizeObserver,
  };
}
