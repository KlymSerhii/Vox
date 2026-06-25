import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import {
  getNotchGeometry,
  getNotchList,
  getPublicSettings,
  notchListMarkDone,
  notchResize,
  onNotchGeometry,
  onNotchListChanged,
  onSettingsChanged,
  type NotchGeometry,
  type NotchItem,
  type NotchListDoc,
} from "../lib/ipc";
import { HotkeyKeys } from "../components/HotkeyKeys";

/// Default top-N if settings haven't loaded yet (matches the backend
/// `default_notch_list_top_n`).
const DEFAULT_TOP_N = 5;
/// Default voice hotkey spec (matches `default_notch_list_voice_hotkey`).
const DEFAULT_VOICE_HOTKEY = "Cmd+Alt+L";
/// Default hover intent delay (matches `default_notch_list_hover_delay_ms`).
const DEFAULT_HOVER_DELAY_MS = 120;

/// Height of the collapsed "show #1 task" bar (populated state). Needs room
/// for a line of text. The window is content-driven via the ResizeObserver,
/// so this is just the row height.
const COLLAPSED_H = 30;
const ROW_H = 28;
const PANEL_PAD = 12;

/* ---------------------------------------------------------------------------
 * Idle (empty) strip geometry — logical px. The empty state renders as one
 * continuous black object that reads as the camera cutout extended downward:
 * [Vox chip] [central cutout] [Notch chip], flush to the window's top edge,
 * square top corners, rounded bottom corners. A hint nub descends from the
 * cutout on hover. See `IdleStrip`.
 * ------------------------------------------------------------------------ */
/// Resting strip height.
const STRIP_H = 34;
/// Fallback central-cutout width when the real camera-bump width is unknown
/// (notchless display, or before geometry resolves).
const CUTOUT_FALLBACK_W = 200;
/// Cutout width clamp. Slightly wider than the physical notch is acceptable.
const CUTOUT_MIN_W = 180;
const CUTOUT_MAX_W = 230;
/// Equal-width side chips ("Vox" / "Notch"). EQUAL width is required so the
/// cutout stays horizontally centered in the (screen-centered) window and
/// therefore lines up under the physical camera notch.
const SIDE_CHIP_W = 78;
/// Outer bottom radius of the two end chips, and the smaller inner radius at
/// the seam where each chip meets the cutout — so the three pieces read as
/// fused-but-articulated.
const CHIP_OUTER_R = 18;
const CHIP_INNER_R = 10;
/// Hint nub (hover only) — descends from the underside of the central cutout.
const HINT_H = 30;
/// Nub is inset from the cutout width so it reads as extruding from it.
const HINT_INSET = 8;
/// Negative top margin: the nub overlaps the cutout's bottom so it reads as
/// extruding from the notch, not a detached tooltip.
const HINT_OVERLAP = 2;
const HINT_BOTTOM_R = 14;

/// Checkbox border + row-hover tints. Hardcoded (not theme tokens) because
/// the notch is pinned to a near-black surface regardless of app appearance,
/// and the default `--color-border` (0.08 white) is too faint on pure black.
const CHECKBOX_BORDER = "rgba(255,255,255,0.32)";

function reducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}

/// Priority dot color. high = red, med = amber, low = muted grey.
function priorityColor(p: NotchItem["priority"]): string {
  switch (p) {
    case "high":
      return "var(--color-danger, #e5484d)";
    case "med":
      return "var(--color-warning, #f5a623)";
    default:
      return "var(--color-ink-faint, #8a8a8a)";
  }
}

export default function NotchView() {
  const [doc, setDoc] = createSignal<NotchListDoc>({
    version: 1,
    items: [],
    archive: [],
  });
  const [hovered, setHovered] = createSignal(false);
  const [topN, setTopN] = createSignal(DEFAULT_TOP_N);
  const [voiceHotkey, setVoiceHotkey] = createSignal(DEFAULT_VOICE_HOTKEY);
  const [hoverDelayMs, setHoverDelayMs] = createSignal(DEFAULT_HOVER_DELAY_MS);
  const [clickToExpand, setClickToExpand] = createSignal(false);
  /// Physical-notch geometry, pushed from Rust. Drives the idle nub width so
  /// it never spills past the camera bump.
  const [geo, setGeo] = createSignal<NotchGeometry>({
    has_notch: false,
    width: 0,
    height: 0,
  });

  let unlistenList: (() => void) | null = null;
  let unlistenSettings: (() => void) | null = null;
  let unlistenGeo: (() => void) | null = null;
  let panelRef: HTMLDivElement | undefined;
  let ro: ResizeObserver | undefined;
  /// Pending hover-intent timer; cleared on leave / before re-arming.
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;

  function clearHoverTimer() {
    if (hoverTimer !== undefined) {
      clearTimeout(hoverTimer);
      hoverTimer = undefined;
    }
  }

  /// Expand after the configured intent delay so the cursor merely
  /// passing over the strip doesn't pop the panel. A 0 ms delay expands
  /// on the next tick. No-op when click-to-expand is on.
  function armHoverExpand() {
    if (clickToExpand()) return;
    clearHoverTimer();
    hoverTimer = setTimeout(() => {
      hoverTimer = undefined;
      setHovered(true);
      queueMicrotask(syncWindowSize);
    }, Math.max(0, hoverDelayMs()));
  }

  /// Idle-strip hint reveal. Reuses the same intent timer and `hoverDelayMs`
  /// as `armHoverExpand`, but is NOT gated by click-to-expand: the empty
  /// state is always hover-driven (the spec's "hover-only + hotkey-only"),
  /// and click-to-expand is a populated-state affordance that has no meaning
  /// when there's nothing to expand.
  function armIdleHint() {
    clearHoverTimer();
    hoverTimer = setTimeout(() => {
      hoverTimer = undefined;
      setHovered(true);
      queueMicrotask(syncWindowSize);
    }, Math.max(0, hoverDelayMs()));
  }

  function collapse() {
    clearHoverTimer();
    setHovered(false);
    queueMicrotask(syncWindowSize);
  }

  /// Toggle expand/collapse on click (click-to-expand mode only). Shared by
  /// the idle nub and the collapsed #1-task bar so both affordances behave
  /// the same.
  function toggleClickExpand() {
    if (!clickToExpand()) return;
    setHovered((v) => !v);
    queueMicrotask(syncWindowSize);
  }

  /// Active items sorted by (priority desc, order asc) — the backend
  /// already returns them sorted in `items`, but we re-sort defensively
  /// so the notch never depends on write-order.
  const activeItems = createMemo(() => {
    const rank = (p: NotchItem["priority"]) =>
      p === "high" ? 0 : p === "med" ? 1 : 2;
    return doc()
      .items.filter((i) => !i.done)
      .slice()
      .sort((a, b) => rank(a.priority) - rank(b.priority) || a.order - b.order);
  });

  const top = createMemo(() => activeItems().slice(0, topN()));
  const isEmpty = createMemo(() => activeItems().length === 0);
  const headItem = createMemo(() => activeItems()[0] ?? null);

  /// Central-cutout width for the idle strip: the real camera-bump width when
  /// known, else a sensible default. Rounded so the window doesn't request
  /// sub-pixel sizes, and clamped to [CUTOUT_MIN_W, CUTOUT_MAX_W] so it tracks
  /// the physical notch (matching, or slightly wider) without a bad geometry
  /// read stretching it to an absurd width.
  const cutoutWidth = createMemo(() => {
    const w = geo().width;
    const base = w >= 80 && w <= 360 ? Math.round(w) : CUTOUT_FALLBACK_W;
    return Math.min(CUTOUT_MAX_W, Math.max(CUTOUT_MIN_W, base));
  });

  async function refresh() {
    try {
      setDoc(await getNotchList());
    } catch (e) {
      console.error("getNotchList:", e);
    }
  }

  async function applySettings() {
    try {
      const s = await getPublicSettings();
      setTopN(Math.min(7, Math.max(3, s.notch_list_top_n || DEFAULT_TOP_N)));
      setVoiceHotkey(s.notch_list_voice_hotkey || DEFAULT_VOICE_HOTKEY);
      setHoverDelayMs(
        Math.min(
          1000,
          Math.max(0, s.notch_list_hover_delay_ms ?? DEFAULT_HOVER_DELAY_MS),
        ),
      );
      const click = s.notch_list_click_to_expand ?? false;
      setClickToExpand(click);
      // Switching into click-to-expand mid-hover should drop any pending
      // hover-intent timer so it can't fire after the mode changed.
      if (click) clearHoverTimer();
    } catch (e) {
      console.warn("getPublicSettings (notch):", e);
    }
  }

  /// Push the measured content size up to the OS window so it grows on
  /// hover and snaps back on leave. Mirrors how the pill drives its size.
  /// The `notch_resize` command re-centers the window below the notch (the
  /// frontend can't know the display origin), so it owns sizing.
  function syncWindowSize() {
    const el = panelRef;
    if (!el) return;
    const w = Math.ceil(el.scrollWidth);
    const h = Math.ceil(el.scrollHeight);
    void notchResize(w, h).catch((e) => console.warn("notchResize:", e));
  }

  onMount(async () => {
    await applySettings();
    try {
      setGeo(await getNotchGeometry());
    } catch (e) {
      console.warn("getNotchGeometry:", e);
    }
    await refresh();
    unlistenList = await onNotchListChanged(setDoc);
    unlistenSettings = await onSettingsChanged(() => {
      void applySettings();
    });
    // The notch can hop displays (cursor-follow) or learn its geometry just
    // after launch — re-size the nub when that happens.
    unlistenGeo = await onNotchGeometry(setGeo);

    if (panelRef && typeof ResizeObserver !== "undefined") {
      ro = new ResizeObserver(() => syncWindowSize());
      ro.observe(panelRef);
    }
    // Initial size sync after first paint.
    queueMicrotask(syncWindowSize);
  });

  onCleanup(() => {
    unlistenList?.();
    unlistenSettings?.();
    unlistenGeo?.();
    ro?.disconnect();
    clearHoverTimer();
  });

  async function markDone(id: string) {
    try {
      await notchListMarkDone(id);
    } catch (e) {
      console.error("notchListMarkDone:", e);
    }
  }

  const transition = () =>
    reducedMotion() ? "none" : "background 120ms ease, opacity 120ms ease";
  /// Reveal transition for the expanded panel content (rows fading in as
  /// the window grows). Disabled under reduced-motion.
  const panelTransition = () =>
    reducedMotion() ? "none" : "opacity 140ms ease, transform 140ms ease";
  /// Checkbox check-off / border colour transition. Disabled under
  /// reduced-motion.
  const checkboxTransition = () =>
    reducedMotion() ? "none" : "border-color 120ms ease, background 120ms ease";

  return (
    // Stable outer wrapper — ALWAYS the same element across empty↔populated
    // swaps so `panelRef` (and the ResizeObserver bound to it in onMount)
    // never goes stale. It is styling-neutral (transparent, shrink-wraps to
    // content); the visible surface is painted by the inner branch (the glass
    // panel for the populated state, the solid-black strip for idle). Mouse
    // handlers live here, branching on empty vs populated so the idle strip is
    // always hover-driven while the populated panel honours click-to-expand.
    <div
      ref={panelRef}
      class="inline-flex w-max max-w-[440px] flex-col items-stretch"
      aria-label={isEmpty() ? "Vox Notch — empty" : "Vox Notch"}
      tabindex={!isEmpty() && clickToExpand() ? 0 : undefined}
      onMouseEnter={() => {
        if (isEmpty()) {
          armIdleHint();
          return;
        }
        if (clickToExpand()) return;
        armHoverExpand();
      }}
      onMouseLeave={() => {
        if (isEmpty()) {
          collapse();
          return;
        }
        if (clickToExpand()) return;
        collapse();
      }}
      onFocusOut={(e) => {
        // Click-to-expand (populated only): collapse when focus leaves the
        // panel (a click elsewhere on the desktop). Hover mode ignores it. We
        // use `focusout` (which bubbles, unlike `blur`) so this still fires
        // when focus was on a descendant — and we ignore moves that stay
        // INSIDE the panel (e.g. clicking a row's "mark done" checkbox), which
        // would otherwise collapse the panel the instant the user ticks the
        // first item.
        if (isEmpty() || !clickToExpand()) return;
        const next = e.relatedTarget as Node | null;
        if (next && panelRef?.contains(next)) return;
        collapse();
      }}
    >
      <Show
        when={!isEmpty()}
        fallback={
          // Empty / idle state — a continuous black object that reads as the
          // camera cutout extended downward, with an on-hover hint nub.
          // Self-contained styling (no glass, no blur, square top corners).
          <IdleStrip
            cutoutWidth={cutoutWidth()}
            hovered={hovered()}
            reduced={reducedMotion()}
            voiceHotkey={voiceHotkey()}
          />
        }
      >
        <div
          class="flex w-full flex-col overflow-hidden rounded-[14px]"
          style={{
            // Always near-black so the panel reads as an extension of the
            // physical (true-black) notch, regardless of app appearance. All
            // tokens here come from the notch's own stylesheet
            // (`src/notch.css`), never the pill/app design system.
            background: "var(--color-notch-surface, rgba(10,10,12,0.92))",
            "backdrop-filter": "var(--notch-blur, blur(20px) saturate(150%))",
            "-webkit-backdrop-filter":
              "var(--notch-blur, blur(20px) saturate(150%))",
            // Faint top rim for definition over dark wallpapers; the OS window
            // shadow (hasShadow=YES) supplies the drop shadow under the panel.
            "box-shadow":
              "inset 0 1px 0 rgba(255,255,255,0.06), 0 8px 24px rgba(0,0,0,0.55)",
            color: "var(--color-ink)",
          }}
        >
          {/* Collapsed resting state: the #1 task line. Expanded: top-N rows. */}
          <Show
            when={!hovered()}
            fallback={
              <div
                class="flex flex-col gap-0.5 px-2 py-1.5"
                style={{
                  "min-height": `${PANEL_PAD + ROW_H}px`,
                  transition: panelTransition(),
                }}
              >
                <For each={top()}>
                  {(item) => (
                    <div
                      class="flex items-center gap-2 rounded-[7px] px-1.5 py-1 hover:bg-white/[0.06]"
                      style={{ height: `${ROW_H}px`, transition: transition() }}
                    >
                      <button
                        type="button"
                        aria-label="Mark done"
                        onClick={(e) => {
                          // Keep a checkbox click from bubbling to a
                          // click-to-expand panel toggle.
                          e.stopPropagation();
                          void markDone(item.id);
                        }}
                        class="flex h-[15px] w-[15px] shrink-0 items-center justify-center rounded-full border"
                        style={{
                          "border-color": CHECKBOX_BORDER,
                          transition: checkboxTransition(),
                        }}
                      />
                      <span
                        class="h-[7px] w-[7px] shrink-0 rounded-full"
                        style={{ background: priorityColor(item.priority) }}
                        aria-hidden="true"
                      />
                      <span class="truncate text-[12px] leading-[16px] text-[var(--color-ink)]">
                        {item.text}
                      </span>
                    </div>
                  )}
                </For>
              </div>
            }
          >
            {/* Click-to-expand wrapper: the #1-task bar expands on click. */}
            <div
              classList={{ "cursor-pointer": clickToExpand() }}
              onClick={toggleClickExpand}
            >
              <Show when={headItem()}>
                {(item) => (
                  <div
                    class="flex items-center gap-2 px-3"
                    style={{ height: `${COLLAPSED_H}px` }}
                  >
                    <span
                      class="h-[7px] w-[7px] shrink-0 rounded-full"
                      style={{ background: priorityColor(item().priority) }}
                      aria-hidden="true"
                    />
                    <span class="truncate text-[12px] leading-[16px] text-[var(--color-ink)]">
                      {item().text}
                    </span>
                  </div>
                )}
              </Show>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
}

/// Shared premium easing for the idle hint reveal.
const EASE_IN = "cubic-bezier(0.32, 0.72, 0, 1)";
/// Sharper ease for retraction.
const EASE_OUT = "cubic-bezier(0.4, 0, 1, 1)";
/// How long to hold the reserved height after hover-out so the retract
/// animation can play before the window collapses in a single step. Slightly
/// over the longest exit transition (the 200ms nub transform).
const RETRACT_MS = 210;

/// Idle / empty state — one continuous black object that reads as the camera
/// cutout extended downward: [Vox chip] [central cutout] [Notch chip], flush
/// to the window's top edge, square top corners, rounded bottom corners. On
/// hover a hint nub descends from the underside of the central cutout carrying
/// the voice-hotkey copy.
///
/// Single-resize discipline: when hover begins, the hint's height is reserved
/// INSTANTLY (no height transition) so `scrollHeight` jumps once and the OS
/// window resizes a single time; only the nub's transform/opacity then animate
/// via CSS inside that reserved space. The transform reveal is armed one
/// animation frame after the reserve so the first frame can't clip during the
/// OS resize round-trip. On hover-out the reserved height is held for one
/// retract duration so the exit animation is visible, then released in a
/// single step (one resize down) — never a chasing/height animation.
function IdleStrip(props: {
  cutoutWidth: number;
  hovered: boolean;
  reduced: boolean;
  voiceHotkey: string;
}) {
  // Two coordinated signals drive a single-resize reveal with a visible
  // retract:
  //   `reserved` — gates the in-flow spacer height AND whether the (absolutely
  //                positioned) nub is mounted. Flips true the instant hover
  //                begins so `scrollHeight` jumps once → one OS resize up. On
  //                hover-out it stays true through the exit animation, then
  //                flips false → one OS resize down. Keeping the nub unmounted
  //                while collapsed is what keeps the invisible nub from
  //                inflating the resting 34px `scrollHeight`.
  //   `armed`    — drives the nub's transform/opacity to their final state. On
  //                the way IN it trails `reserved` by one frame so the reveal
  //                can't clip during the OS resize round-trip; on the way OUT
  //                it drops immediately so the retract animation plays first,
  //                inside the still-reserved height.
  const [reserved, setReserved] = createSignal(false);
  const [armed, setArmed] = createSignal(false);
  let raf = 0;
  let collapseTimer: ReturnType<typeof setTimeout> | undefined;
  const clearCollapse = () => {
    if (collapseTimer !== undefined) {
      clearTimeout(collapseTimer);
      collapseTimer = undefined;
    }
  };
  createEffect(() => {
    if (props.hovered) {
      clearCollapse();
      setReserved(true);
      if (props.reduced) {
        setArmed(true);
      } else {
        if (raf) cancelAnimationFrame(raf);
        raf = requestAnimationFrame(() => {
          raf = 0;
          setArmed(true);
        });
      }
    } else {
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      setArmed(false);
      // Hold the reserved height through the retract, then collapse in one
      // step. Reduced motion skips the wait and snaps down immediately.
      clearCollapse();
      if (props.reduced) {
        setReserved(false);
      } else {
        collapseTimer = setTimeout(() => {
          collapseTimer = undefined;
          setReserved(false);
        }, RETRACT_MS);
      }
    }
  });
  onCleanup(() => {
    if (raf) cancelAnimationFrame(raf);
    clearCollapse();
  });

  const totalWidth = () => SIDE_CHIP_W * 2 + props.cutoutWidth;
  const nubWidth = () => props.cutoutWidth - HINT_INSET;
  // Reserved hint height, added BELOW the strip via an in-flow spacer. It
  // toggles instantly (no height transition) so `scrollHeight` jumps exactly
  // once and the OS window resizes a single time. The nub itself is absolutely
  // positioned (overlapping the strip's bottom by HINT_OVERLAP) and so doesn't
  // participate in this height — total growth stays HINT_H - HINT_OVERLAP
  // (~28px → strip 34 + 28 = ~62px tall expanded).
  const reservedH = () => (reserved() ? HINT_H - HINT_OVERLAP : 0);

  // Side-word opacity: subtle at idle, brighter on hover.
  const wordOpacity = () => (props.hovered ? 0.9 : 0.55);
  const wordTransition = () =>
    props.reduced ? "opacity 100ms linear" : `opacity 180ms ${EASE_IN}`;

  // Nub transform/opacity. Reduced motion: opacity-only cross-fade, no scale
  // or translate (and the height jump is already instant).
  const nubTransform = () => {
    if (props.reduced) return "none";
    return armed() ? "scaleY(1) translateY(0)" : "scaleY(0.4) translateY(-6px)";
  };
  const nubOpacity = () => (armed() ? 1 : 0);
  const nubTransition = () => {
    if (props.reduced) return "opacity 100ms linear";
    return props.hovered
      ? // in: transform 320ms; opacity 0→1 delayed ~40ms
        `transform 320ms ${EASE_IN}, opacity 280ms ${EASE_IN} 40ms`
      : // out: retract + fade ~120ms, sharper ease
        `transform 200ms ${EASE_OUT}, opacity 120ms ${EASE_OUT}`;
  };

  // Hint text trails the nub: fades + rises (translateY 2px→0) starting ~90ms.
  const textOpacity = () => (armed() ? 1 : 0);
  const textTransform = () =>
    props.reduced || armed() ? "translateY(0)" : "translateY(2px)";
  const textTransition = () => {
    if (props.reduced) return "opacity 100ms linear";
    return props.hovered
      ? `opacity 230ms ${EASE_IN} 90ms, transform 230ms ${EASE_IN} 90ms`
      : `opacity 120ms ${EASE_OUT}, transform 120ms ${EASE_OUT}`;
  };

  return (
    <div
      class="relative flex flex-col items-center"
      style={{ width: `${totalWidth()}px` }}
    >
      {/* The black strip: three fused segments, flush to the window top. */}
      <div class="flex w-full" style={{ height: `${STRIP_H}px` }}>
        {/* Left chip — "Vox" */}
        <div
          class="flex items-center justify-center"
          style={{
            width: `${SIDE_CHIP_W}px`,
            background: "#000",
            "border-radius": `0 0 ${CHIP_INNER_R}px ${CHIP_OUTER_R}px`,
            // Subtle bezel realism on the underside; no outer border (it would
            // break the cutout illusion — the OS drop shadow supplies depth).
            "box-shadow": "inset 0 -0.5px 0 rgba(255,255,255,0.04)",
          }}
        >
          <span
            style={{
              "font-size": "14px",
              "font-weight": 700,
              "letter-spacing": "-0.01em",
              color: "#fff",
              opacity: `${wordOpacity()}`,
              transition: wordTransition(),
            }}
          >
            Vox
          </span>
        </div>

        {/* Central cutout — pure-black continuation of the physical notch. */}
        <div
          aria-hidden="true"
          style={{
            width: `${props.cutoutWidth}px`,
            background: "#000",
            // Square all corners: this segment IS the notch, continued down.
            "border-radius": "0",
          }}
        />

        {/* Right chip — "Notch" */}
        <div
          class="flex items-center justify-center"
          style={{
            width: `${SIDE_CHIP_W}px`,
            background: "#000",
            "border-radius": `0 0 ${CHIP_OUTER_R}px ${CHIP_INNER_R}px`,
            "box-shadow": "inset 0 -0.5px 0 rgba(255,255,255,0.04)",
          }}
        >
          <span
            style={{
              "font-size": "14px",
              "font-weight": 500,
              "letter-spacing": "-0.01em",
              color: "#fff",
              opacity: `${wordOpacity()}`,
              transition: wordTransition(),
            }}
          >
            Notch
          </span>
        </div>
      </div>

      {/* In-flow spacer: reserves the hint's height so `scrollHeight` jumps
          once (single OS resize). Instant — no height transition. */}
      <div style={{ height: `${reservedH()}px` }} aria-hidden="true" />

      {/* Hint nub — absolutely positioned so it can overlap the strip's
          underside (HINT_OVERLAP) without being clipped and without feeding
          the height animation. Anchored just inside the strip's bottom edge,
          horizontally centered → sits directly under the central cutout. Only
          mounted while the height is reserved, so the invisible nub can't
          inflate the resting `scrollHeight`. */}
      <Show when={reserved()}>
        <div
          class="pointer-events-none absolute inset-x-0 flex justify-center"
          style={{ top: `${STRIP_H - HINT_OVERLAP}px` }}
          aria-hidden={props.hovered ? undefined : "true"}
        >
          <div
            class="flex items-center justify-center"
            style={{
              width: "max-content",
              "min-width": `${nubWidth()}px`,
              "max-width": `${totalWidth()}px`,
              height: `${HINT_H}px`,
              background: "#000",
              // Square top (continues the cutout), rounded bottom.
              "border-radius": `0 0 ${HINT_BOTTOM_R}px ${HINT_BOTTOM_R}px`,
              padding: "0 16px",
              "transform-origin": "top center",
              transform: nubTransform(),
              opacity: `${nubOpacity()}`,
              transition: nubTransition(),
              "will-change": "transform, opacity",
            }}
          >
            <div
              class="flex items-center gap-1.5 whitespace-nowrap"
              style={{
                "font-size": "11px",
                "font-weight": 500,
                "letter-spacing": "0.01em",
                color: "rgba(255,255,255,0.62)",
                opacity: `${textOpacity()}`,
                transform: textTransform(),
                transition: textTransition(),
              }}
            >
              <HotkeyKeys spec={props.voiceHotkey} />
              <span>to add a task by voice</span>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
}
