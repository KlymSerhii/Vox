import {
  createMemo,
  createSignal,
  For,
  Index,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import {
  getNotchGeometry,
  getNotchList,
  getPublicSettings,
  notchFocusInput,
  notchListAdd,
  notchListMarkDone,
  notchListReorder,
  notchListSetPriority,
  notchReleaseInputFocus,
  notchResize,
  onNotchGeometry,
  onNotchHover,
  onNotchLevels,
  onNotchListChanged,
  onNotchListening,
  onSettingsChanged,
  type NotchGeometry,
  type NotchItem,
  type NotchListDoc,
  type NotchPriority,
} from "../lib/ipc";
import { formatHotkeyTokens } from "../lib/hotkey";

/// Default top-N if settings haven't loaded yet (matches the backend
/// `default_notch_list_top_n`).
const DEFAULT_TOP_N = 5;
/// Default hover intent delay (matches `default_notch_list_hover_delay_ms`).
const DEFAULT_HOVER_DELAY_MS = 120;

/// Expanded-row height (drag handle + checkbox + priority dot + text).
const ROW_H = 28;
/// Vertical padding inside the expanded list (top + bottom).
const LIST_PAD_Y = 6;
/// Collapsed single-line drawer height (the #1-task bar). Kept at 16px (a slim
/// glanceable strip the user liked) so the idle slab hangs as little as
/// possible below the camera bump — at the old 30px it overlapped app chrome
/// right under the menu bar (e.g. browser tabs). The idle task sits at the
/// small (8px) word size, dimmed and centered, within this band.
const COLLAPSED_H = 16;
/// Add-task hint row height — the keycap affordance. Full size as the faint
/// FOOTER beneath a non-empty hovered list.
const HINT_H = 30;
/// Compact add-task hint height for the EMPTY + hovered state, where the
/// keycap affordance is the sole drawer content. Under half the old 30px (and
/// a hair under the task bar) so hovering an empty notch no longer hangs over
/// app chrome, while still teaching ⌘⌥L via the smaller `.notch-kbd-sm` chips.
const EMPTY_HINT_H = 14;
/// Quick-add composer row height — the typed-task input + priority picker that
/// replaces the add-task hint while the user is composing. A touch taller than
/// HINT_H so the input has comfortable vertical room.
const COMPOSE_H = 36;
/// Grace window (ms) after the composer opens during which a transient `blur`
/// is treated as the focus handshake (re-grab the input) rather than a real
/// dismiss. Covers the brief gap before `set_focus` makes the notch key.
const COMPOSE_BLUR_GRACE_MS = 300;

/* ---------------------------------------------------------------------------
 * Ear-row + cutout geometry — logical px. The window is TOP-ANCHORED at the
 * screen's top edge (Rust owns the y-anchor + re-centering). The view is one
 * vertical stack:
 *
 *   [ear-row]   <- always visible: [Vox ear][black spacer][Notch ear]
 *   [drawer]    <- below the cutout: 0-height when idle+empty; otherwise a
 *                  connected slab (collapsed task / hint / voice / list)
 *
 * The ear row is width-pinned (earW*2 + cutoutWidth) and centered so the black
 * spacer always sits under the physical camera bump. Both the ears and the
 * drawer are hover-responsive: idle they're a slim nub; on hover the ears snap
 * to full size and the drawer fans out to the task list. The connected slab
 * (every non-`list` drawer state) matches the ear-row width so it reads as one
 * black panel rather than a narrow tab under the bump.
 * ------------------------------------------------------------------------ */

/// Fallback central-cutout width when the real camera-bump width is unknown
/// (notchless display, or before geometry resolves).
const CUTOUT_FALLBACK_W = 200;
/// Cutout width clamp. Slightly wider than the physical notch is acceptable.
const CUTOUT_MIN_W = 180;
const CUTOUT_MAX_W = 230;
/// Cutout-height (menu-bar band) clamp + fallback. `geo.height` is the
/// safeAreaInsets.top band (~32–38pt). The ears match this so their top edge
/// forms one unbroken line with the physical bump.
const CUTOUT_H_FALLBACK = 34;
const CUTOUT_H_MIN = 24;
const CUTOUT_H_MAX = 48;

/// Ear width clamp — HOVER (full) size. Seats the longer "Notch" label with
/// comfortable padding at the hovered 11px word size, tied to the cutout so
/// the assembly stays proportional across notch geometries.
const EAR_W_MIN = 46;
const EAR_W_MAX = 56;
/// Ear width clamp — IDLE (resting) size. The ears shrink to a slim nub when
/// the notch isn't hovered (smaller word size), then snap back to the full
/// size above on hover. Sized to seat "Notch" at the idle 8px word size.
const EAR_W_IDLE_MIN = 28;
const EAR_W_IDLE_MAX = 33;
/// Word size in the ears: slim when idle, full (the original) on hover. Snaps
/// with the ear width (no size transition) so the wrapper still measures once
/// per hover edge — the smooth hover feedback is the opacity fade below.
const EAR_FONT_IDLE = 8;
const EAR_FONT_HOVER = 11;

/// Expanded-drawer width clamp (relative to the cutout width).
const EXPANDED_W_MIN = 280;
const EXPANDED_W_MAX = 380;

/// Checkbox border. Hardcoded (not a theme token) because the notch sits on
/// pure black regardless of app appearance.
const CHECKBOX_BORDER = "rgba(255,255,255,0.32)";

/// Listening-waveform bar fill — a cool silver shimmer (a hair of blue) that
/// reads as a deliberate "tech/listening" accent on the pure-black panel,
/// calmer and unmistakably distinct from the pill's coral. Hardcoded for the
/// same reason as the checkbox border: the notch is always on true black. The
/// 0.70 base alpha keeps the thin (1.5px) edge-dissolved bars from vanishing
/// once the sine envelope × live amplitude knock them down.
const WAVE_COLOR = "rgba(207, 216, 224, 0.7)";

/// Shared premium easing for reveals.
const EASE_IN = "cubic-bezier(0.32, 0.72, 0, 1)";
/// Overshoot ease for the drag-settle.
const EASE_SETTLE = "cubic-bezier(0.34, 1.4, 0.64, 1)";
/// Duration for the fluid size animations — the hover grow/shrink of the ears
/// and the drawer box. The window CHASES this (a rAF-coalesced ResizeObserver
/// drives `notch_resize` per frame) so the OS panel glides instead of snapping.
const NOTCH_ANIM_MS = 260;

/// Task-completion choreography (checkbox → strike-through → undo → remove).
/// `STRIKE_MS`: how long the line takes to draw across the text. `UNDO_MS`: the
/// window the Undo button stays clickable (unless the user leaves first).
/// `EXIT_MS`: the row's collapse-and-fade once it finally commits.
const STRIKE_MS = 1600;
const UNDO_MS = 5000;
/// Kept just under NOTCH_ANIM_MS so the row finishes collapsing a hair before
/// the drawer settles — any momentary mismatch is an invisible black gap at the
/// bottom rather than a visible clip of the footer.
const EXIT_MS = 240;
/// Change-flash duration (keep in sync with the `.notch-flash` keyframe).
const FLASH_MS = 400;
/// How long an `error` phase (mis-heard / silence / clarify / confirm prompt)
/// stays on screen before the drawer auto-returns to its idle/list state.
/// Rust does not follow `error` with `idle`, so this is the only thing that
/// clears it (and restores hover-expand). Long enough to read a confirm
/// prompt, short enough not to feel stuck.
const ERROR_AUTO_CLEAR_MS = 4500;
/// Drag threshold before a pointerdown becomes a lift.
const DRAG_THRESHOLD = 4;
/// While a lifted row's pointer is within this many px of the scroll viewport's
/// top/bottom edge, the list auto-scrolls so below-the-fold slots are reachable.
const DRAG_EDGE_PX = 22;
/// Per-frame auto-scroll step (px) while the pointer sits in the edge band.
const DRAG_SCROLL_STEP = 7;
/// After the user stops speaking, STT runs but `phase` is still "listening"
/// and `notch:levels` stop arriving — the bars would freeze mid-amplitude.
/// Once levels go stale past this window we ease them down to a calm resting
/// state so the ribbon reads as "settled", not stuck.
const LEVELS_STALE_MS = 400;
/// Decay tick cadence + per-tick multiplier for the stale-levels ease-down.
const LEVELS_DECAY_MS = 60;
const LEVELS_DECAY_FACTOR = 0.82;

type DrawerPhase = "listening" | "compiling" | "error";

function reducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}

/// Priority dot color. high = red, med = amber, low = muted grey. Used in the
/// bright hovered list.
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

/// Dimmed priority dot for the idle (collapsed) task bar — desaturated toward
/// the ear-grey ramp so the single glanceable task reads as calm/ambient
/// rather than an alert. The vibrant `priorityColor()` stays for the hovered
/// expanded list, where the user is actively triaging.
function priorityColorDim(p: NotchItem["priority"]): string {
  switch (p) {
    case "high":
      return "#a85a5c";
    case "med":
      return "#b08a4a";
    default:
      return "var(--color-ink-faint, rgba(255,255,255,0.4))";
  }
}

/// Soft sine bell — 0 at the edges, 1 in the middle. Mirrors
/// `pill/helpers.ts::edgeEnvelope` so the notch waveform dissolves into the
/// black at its edges exactly like the pill's does.
function edgeEnvelope(i: number, n: number): number {
  const t = (i + 0.5) / n;
  return Math.sin(Math.PI * t);
}

export default function NotchView() {
  const [doc, setDoc] = createSignal<NotchListDoc>({
    version: 1,
    items: [],
    archive: [],
  });
  /// `true` while the cursor is over the physical notch rect. Driven by the
  /// `notch:hover` event (authoritative — fires even when another app is
  /// focused) with DOM enter/leave as a secondary trigger.
  const [hovered, setHovered] = createSignal(false);
  const [topN, setTopN] = createSignal(DEFAULT_TOP_N);
  const [hoverDelayMs, setHoverDelayMs] = createSignal(DEFAULT_HOVER_DELAY_MS);
  // Task-completion limbo state — declared here (before the `headItem` memo,
  // which excludes `exitingIds`). The functions that drive these live further
  // down by `commitTask`.
  const [completingIds, setCompletingIds] = createSignal<Set<string>>(
    new Set(),
  );
  const [exitingIds, setExitingIds] = createSignal<Set<string>>(new Set());
  const undoTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const exitTimers = new Map<string, ReturnType<typeof setTimeout>>();
  /// The notch voice hotkey (default ⌥⌘L), shown in the on-hover add-task
  /// hint. Kept in sync via `applySettings` / `settings:changed`.
  const [voiceHotkey, setVoiceHotkey] = createSignal<string>("Cmd+Alt+L");
  /// Physical-notch geometry, pushed from Rust.
  const [geo, setGeo] = createSignal<NotchGeometry>({
    has_notch: false,
    width: 0,
    height: 0,
  });

  // --- Quick-add composer (typed task entry) -----------------------------
  /// True while the in-notch typed-task input is open. Keeps the drawer
  /// expanded even if the cursor leaves (the user is typing, not hovering).
  const [composing, setComposing] = createSignal(false);
  const [newText, setNewText] = createSignal("");
  const [newPriority, setNewPriority] = createSignal<NotchPriority>("med");
  /// The live composer <input>, for programmatic focus/refocus.
  let composerInput: HTMLInputElement | undefined;
  /// Armed COMPOSE_BLUR_GRACE_MS after the composer opens — until then a blur
  /// is the focus handshake (re-grab), after it a real dismiss (close).
  let composeBlurArmed = false;
  let composeArmTimer: ReturnType<typeof setTimeout> | undefined;

  // --- Voice (⌘L) drawer state -------------------------------------------
  /// Current voice phase, or null when not capturing. Drives the waveform /
  /// compiling / error drawer.
  const [phase, setPhase] = createSignal<DrawerPhase | null>(null);
  const [phaseMessage, setPhaseMessage] = createSignal<string>("");
  const [levels, setLevels] = createSignal<number[]>(new Array(32).fill(0));
  /// Set of item ids to flash once after a voice edit lands.
  const [flashIds, setFlashIds] = createSignal<Set<string>>(new Set());

  // --- Drag-to-reorder state ---------------------------------------------
  /// Id of the row being dragged, or null. The dragged row lifts and follows
  /// the pointer; siblings slide via a computed gap.
  const [dragId, setDragId] = createSignal<string | null>(null);
  /// Pointer-following Y offset (px) of the lifted row from its origin slot.
  const [dragOffsetY, setDragOffsetY] = createSignal(0);
  /// Target insertion index (into the active-id order) while dragging.
  const [dropIndex, setDropIndex] = createSignal<number | null>(null);
  /// True once the pointer has moved past the threshold (an actual lift, not
  /// a click). Until then the row sits in place so a tap still ticks nothing.
  const [dragLifted, setDragLifted] = createSignal(false);
  /// Id of the just-dropped row during its settle window — drives the
  /// overshoot ease on the transform back to its resting slot.
  const [settleId, setSettleId] = createSignal<string | null>(null);

  let unlistenList: (() => void) | null = null;
  let unlistenSettings: (() => void) | null = null;
  let unlistenGeo: (() => void) | null = null;
  let unlistenHover: (() => void) | null = null;
  let unlistenListening: (() => void) | null = null;
  let unlistenLevels: (() => void) | null = null;
  let panelRef: HTMLDivElement | undefined;
  let listRef: HTMLDivElement | undefined;
  let ro: ResizeObserver | undefined;
  /// rAF handle coalescing a burst of ResizeObserver ticks into one
  /// `syncWindowSize` per frame while the box is mid-animation.
  let resizeRaf: number | null = null;
  /// Pending hover-intent timer; cleared on leave / before re-arming.
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  /// Pending change-flash clear.
  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  /// Pending error-phase auto-clear. Rust emits `error` for every
  /// mis-heard / silence / clarify / destructive-confirm outcome and
  /// (intentionally) does NOT follow with `idle`, so the drawer would pin on
  /// the error text forever — which also keeps `voiceActive()` true and blocks
  /// hover-expand. This timer returns the drawer to its idle/list state.
  let errorTimer: ReturnType<typeof setTimeout> | undefined;
  /// Pointer-drag bookkeeping (non-reactive — set during a gesture).
  let dragStartY = 0;
  let dragStartIndex = 0;
  let dragRowH = ROW_H;
  /// Pointer Y (CONTENT space — viewport-top-relative + scrollTop) at grab, so
  /// drag math survives the list scrolling under the pointer during a drag.
  let dragStartContentY = 0;
  /// Last pointer Y (screen space), read by the edge auto-scroll ticker.
  let dragLastClientY = 0;
  /// rAF handle for the edge auto-scroll loop while a row is lifted.
  let autoScrollRaf: number | null = null;
  /// Pending settle-state clear.
  let settleTimer: ReturnType<typeof setTimeout> | undefined;
  /// Timestamp (ms) of the last `notch:levels` payload, for stale-detection.
  let lastLevelsAt = 0;
  /// Recurring decay ticker that eases frozen bars down once levels go stale.
  let levelsDecayTimer: ReturnType<typeof setInterval> | undefined;

  function clearHoverTimer() {
    if (hoverTimer !== undefined) {
      clearTimeout(hoverTimer);
      hoverTimer = undefined;
    }
  }

  function clearErrorTimer() {
    if (errorTimer !== undefined) {
      clearTimeout(errorTimer);
      errorTimer = undefined;
    }
  }

  /// Expand after the configured intent delay so the cursor merely passing
  /// over the notch doesn't pop the drawer. A 0 ms delay expands next tick.
  function armHoverExpand() {
    clearHoverTimer();
    hoverTimer = setTimeout(() => {
      hoverTimer = undefined;
      setHovered(true);
      queueMicrotask(syncWindowSize);
    }, Math.max(0, hoverDelayMs()));
  }

  function collapse() {
    clearHoverTimer();
    // Leaving the notch confirms any in-flight task completions — there's no
    // way to reach the Undo button once the panel is shut. EXCEPT while
    // composing: the drawer stays open (so Undo is still reachable), so we must
    // NOT commit yet — `endCompose` handles the commit when the drawer really
    // closes.
    if (!composing()) commitPendingCompletions();
    setHovered(false);
    queueMicrotask(syncWindowSize);
  }

  // --- Quick-add composer -------------------------------------------------
  /// Open the typed-task input. Makes the notch the key window (so keystrokes
  /// reach the input) without activating Vox — the click that opened the
  /// composer usually already did, but `notchFocusInput` covers every path and
  /// stashes the prior frontmost app for Escape. Blur-to-close is armed only
  /// after the focus handshake settles so the initial grab can't self-close.
  function startCompose() {
    if (voiceActive()) return;
    setNewText("");
    setNewPriority("med");
    setComposing(true);
    composeBlurArmed = false;
    if (composeArmTimer !== undefined) clearTimeout(composeArmTimer);
    void notchFocusInput().catch((e) => console.warn("notchFocusInput:", e));
    composeArmTimer = setTimeout(() => {
      composeArmTimer = undefined;
      composeBlurArmed = true;
    }, COMPOSE_BLUR_GRACE_MS);
    queueMicrotask(syncWindowSize);
  }

  /// Close the composer. `release` hands keyboard focus back to the app that
  /// was frontmost before composing (the Escape path); a blur-driven close
  /// passes `false` because whatever the user clicked already took focus.
  function endCompose(release: boolean) {
    if (composeArmTimer !== undefined) {
      clearTimeout(composeArmTimer);
      composeArmTimer = undefined;
    }
    composeBlurArmed = false;
    setComposing(false);
    setNewText("");
    // If the cursor already left the notch, closing the composer is what
    // actually shuts the drawer — so commit any completions still in limbo now
    // (their Undo is about to disappear), mirroring `collapse()`.
    if (!hovered()) commitPendingCompletions();
    if (release) {
      void notchReleaseInputFocus().catch((e) =>
        console.warn("notchReleaseInputFocus:", e),
      );
    }
    queueMicrotask(syncWindowSize);
  }

  /// Commit the typed task and keep the input open + focused for rapid
  /// multi-add. The list refreshes via the `notch-list:changed` echo.
  function submitCompose() {
    const text = newText().trim();
    if (!text) return;
    void notchListAdd(text, newPriority()).catch((e) =>
      console.error("notchListAdd:", e),
    );
    setNewText("");
    composerInput?.focus();
    queueMicrotask(syncWindowSize);
  }

  function onComposerBlur() {
    // During the initial focus handshake, a transient blur can fire before
    // `makeKeyAndOrderFront` settles. Re-grab ONLY if our window is still the
    // key window (`document.hasFocus()`); if the user actually clicked into
    // another app (we lost key), close instead of fighting for focus back.
    if (!composeBlurArmed) {
      if (typeof document !== "undefined" && document.hasFocus()) {
        requestAnimationFrame(() => composerInput?.focus());
        return;
      }
    }
    // A real blur (the user clicked into another app/window, which already took
    // key focus) closes the composer without re-activating anything.
    endCompose(false);
  }

  /// Bind + auto-focus the composer <input> the moment it mounts.
  function bindComposerInput(el: HTMLInputElement) {
    composerInput = el;
    requestAnimationFrame(() => el.focus());
  }

  /// Active items sorted by (priority desc, order asc). The backend already
  /// returns them sorted; we re-sort defensively so the notch never depends
  /// on write-order.
  const activeItems = createMemo(() => {
    const rank = (p: NotchItem["priority"]) =>
      p === "high" ? 0 : p === "med" ? 1 : 2;
    return doc()
      .items.filter((i) => !i.done)
      .slice()
      .sort((a, b) => rank(a.priority) - rank(b.priority) || a.order - b.order);
  });

  // Every active item is rendered (not just the top-N): the expanded list is a
  // bounded, scrollable viewport, so overflow scrolls into view rather than
  // being clipped away. `topN` now caps the viewport HEIGHT (rows shown before
  // scrolling), not which items exist.
  const rows = createMemo(() => activeItems());
  const isEmpty = createMemo(() => activeItems().length === 0);
  // The idle collapsed bar shows the top task that ISN'T mid-completion, so a
  // task ticked/committed just before leaving doesn't flash there while its echo
  // is in flight.
  const headItem = createMemo(
    () =>
      activeItems().find(
        (i) => !exitingIds().has(i.id) && !completingIds().has(i.id),
      ) ?? null,
  );

  /// Central-cutout width: real camera-bump width when known, else fallback,
  /// clamped so a bad geometry read can't stretch it absurdly wide.
  const cutoutWidth = createMemo(() => {
    const w = geo().width;
    const base = w >= 80 && w <= 360 ? Math.round(w) : CUTOUT_FALLBACK_W;
    return Math.min(CUTOUT_MAX_W, Math.max(CUTOUT_MIN_W, base));
  });

  /// Cutout / menu-bar-band height. Drives the ear height so the top edge is
  /// one unbroken line with the physical bump.
  const cutoutHeight = createMemo(() => {
    const h = geo().height;
    const base = h >= 16 && h <= 80 ? Math.round(h) : CUTOUT_H_FALLBACK;
    return Math.min(CUTOUT_H_MAX, Math.max(CUTOUT_H_MIN, base));
  });

  /// Ear width — hover-responsive. Idle it's a slim nub (~0.15× cutout); on
  /// hover it snaps to the full size (~0.24×, the original) so the grown 11px
  /// label keeps padding. A hard style value (no CSS width transition) so the
  /// wrapper's measured size still steps exactly once per hover edge.
  const earWidth = createMemo(() => {
    const hov = hovered();
    const factor = hov ? 0.24 : 0.15;
    const lo = hov ? EAR_W_MIN : EAR_W_IDLE_MIN;
    const hi = hov ? EAR_W_MAX : EAR_W_IDLE_MAX;
    return Math.round(Math.min(hi, Math.max(lo, cutoutWidth() * factor)));
  });

  /// Width of the expanded list drawer.
  const expandedWidth = createMemo(() =>
    Math.round(
      Math.min(
        EXPANDED_W_MAX,
        Math.max(EXPANDED_W_MIN, cutoutWidth() * 1.6),
      ),
    ),
  );

  const hasNotch = createMemo(() => geo().has_notch);

  /// Full ear-row width = both ears + the camera-bump spacer. The connected
  /// drawer (collapsed task / empty-hover hint / voice strip) matches this so
  /// it reads as ONE black slab flush under the ears rather than a narrow tab
  /// under the bump. Notchless has no bump to flank, so the slab falls back to
  /// the cutout width (preserving the pre-existing notchless layout).
  const bridgeW = createMemo(() =>
    hasNotch() ? earWidth() * 2 + cutoutWidth() : cutoutWidth(),
  );

  /// Drawer shows the voice strip whenever a phase is active.
  const voiceActive = createMemo(() => phase() !== null);
  /// Hover-expanded drawer: any time the notch is hovered (or the user is
  /// typing a new task) and not mid-voice. Drives both the task list (when
  /// there are items) and the add-task hint / composer. `composing` keeps the
  /// drawer open even after the cursor leaves, so typing isn't interrupted by
  /// the hover-collapse.
  const hoverExpanded = createMemo(
    () => (hovered() || composing()) && !voiceActive(),
  );

  /// Whether a drawer hangs below the ears right now (everything except an
  /// empty, un-hovered, idle notch). When true the ear bottoms square off so
  /// they merge into the connected slab; when false the ears round their
  /// bottom corners into a standalone resting nub.
  const drawerPresent = createMemo(
    () => !isEmpty() || hovered() || voiceActive() || composing(),
  );

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
      setHoverDelayMs(
        Math.min(
          1000,
          Math.max(0, s.notch_list_hover_delay_ms ?? DEFAULT_HOVER_DELAY_MS),
        ),
      );
      setVoiceHotkey(s.notch_list_voice_hotkey || "Cmd+Alt+L");
    } catch (e) {
      console.warn("getPublicSettings (notch):", e);
    }
  }

  /// Push the measured content size up to the OS window so it grows on hover
  /// and shrinks on leave. `notch_resize` re-centers the window below the notch
  /// (the frontend can't know the display origin), so it owns sizing.
  ///
  /// Reads the RENDERED rect (`getBoundingClientRect`) rather than `scrollWidth`
  /// so it tracks the live, mid-transition size: with the ears + drawer now CSS
  /// -animating their width/height, the ResizeObserver fires every frame and
  /// this CHASES the box so the OS window glides in lockstep (the pill's proven
  /// model) instead of the old single snap. A 1px dedup skips redundant IPC once
  /// the size settles.
  let lastSyncW = -1;
  let lastSyncH = -1;
  function syncWindowSize() {
    const el = panelRef;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const w = Math.ceil(r.width);
    const h = Math.ceil(r.height);
    // Exact dedup. No feedback loop: the panel shrink-wraps to explicit-px
    // content (ear widths, drawer box), so its measured size is independent of
    // the window/viewport `notch_resize` sets — a fixed layout measures the same
    // every frame and settles in one tick once the CSS transition ends.
    if (w === lastSyncW && h === lastSyncH) return;
    lastSyncW = w;
    lastSyncH = h;
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
    unlistenList = await onNotchListChanged((next) => {
      flashChangedRows(doc(), next);
      setDoc(next);
      // Reconcile completion bookkeeping against the new list (clears rows whose
      // markDone just landed; drops anything deleted elsewhere).
      pruneCompletionState(next);
    });
    unlistenSettings = await onSettingsChanged(() => {
      void applySettings();
    });
    // The notch can hop displays (cursor-follow) or learn its geometry just
    // after launch — re-measure when that happens.
    unlistenGeo = await onNotchGeometry((g) => {
      setGeo(g);
      queueMicrotask(syncWindowSize);
    });
    // Authoritative hover, hit-tested by Rust against the physical notch rect
    // (works even when another app is focused — the bug we're fixing).
    unlistenHover = await onNotchHover((over) => {
      if (over) armHoverExpand();
      else collapse();
    });
    // ⌘L voice flow.
    unlistenListening = await onNotchListening((s) => {
      const p = s.phase;
      // Any new phase supersedes a pending error auto-clear.
      clearErrorTimer();
      // Voice capture supersedes the typed quick-add — close it and hand the
      // keyboard back so the two input modes never fight over the drawer.
      if (p !== "idle" && composing()) endCompose(true);
      if (p === "idle") {
        setPhase(null);
        setPhaseMessage("");
      } else {
        setPhase(p as DrawerPhase);
        setPhaseMessage(s.message ?? "");
        // Reset the stale-levels clock when listening (re)starts so the
        // decay ticker waits a full window before easing the bars down.
        if (p === "listening") lastLevelsAt = Date.now();
        // `error` is terminal on the Rust side (no following `idle`), so it
        // would otherwise pin the drawer and keep `voiceActive()` true —
        // disabling hover-expand. Auto-clear it back to the idle/list state;
        // this also dismisses the destructive-confirm prompt after the window.
        if (p === "error") {
          errorTimer = setTimeout(() => {
            errorTimer = undefined;
            setPhase(null);
            setPhaseMessage("");
            queueMicrotask(syncWindowSize);
          }, ERROR_AUTO_CLEAR_MS);
        }
      }
      queueMicrotask(syncWindowSize);
    });
    unlistenLevels = await onNotchLevels((lv) => {
      // Coerce to length 32 so a stray payload can't break the waveform.
      if (Array.isArray(lv) && lv.length === 32) {
        lastLevelsAt = Date.now();
        setLevels(lv);
      }
    });

    // Stale-levels ease-down: when STT runs after speech ends, `notch:levels`
    // stop arriving while `phase` is still "listening". Decay the bars toward
    // a calm low floor so the ribbon doesn't look frozen. The CSS `height`
    // transition on each bar smooths the steps. No-op unless mid-listening
    // AND levels are stale, so it never fights live input.
    levelsDecayTimer = setInterval(() => {
      if (phase() !== "listening") return;
      if (Date.now() - lastLevelsAt < LEVELS_STALE_MS) return;
      const cur = levels();
      let anyAbove = false;
      const next = cur.map((v) => {
        const d = v * LEVELS_DECAY_FACTOR;
        const settled = d < 0.02 ? 0 : d;
        if (settled > 0) anyAbove = true;
        return settled;
      });
      if (anyAbove) setLevels(next);
    }, LEVELS_DECAY_MS);

    if (panelRef && typeof ResizeObserver !== "undefined") {
      // Coalesce a burst of observer ticks (the CSS size transitions fire one
      // per animation frame) into a single `syncWindowSize` per frame so the
      // window chase stays smooth without flooding the IPC bridge.
      ro = new ResizeObserver(() => {
        if (resizeRaf !== null) cancelAnimationFrame(resizeRaf);
        resizeRaf = requestAnimationFrame(() => {
          resizeRaf = null;
          syncWindowSize();
        });
      });
      ro.observe(panelRef);
    }
    // Initial size sync after first paint.
    queueMicrotask(syncWindowSize);
  });

  onCleanup(() => {
    unlistenList?.();
    unlistenSettings?.();
    unlistenGeo?.();
    unlistenHover?.();
    unlistenListening?.();
    unlistenLevels?.();
    ro?.disconnect();
    if (resizeRaf !== null) cancelAnimationFrame(resizeRaf);
    clearHoverTimer();
    clearErrorTimer();
    if (flashTimer !== undefined) clearTimeout(flashTimer);
    if (settleTimer !== undefined) clearTimeout(settleTimer);
    if (composeArmTimer !== undefined) clearTimeout(composeArmTimer);
    if (levelsDecayTimer !== undefined) clearInterval(levelsDecayTimer);
    undoTimers.forEach((t) => clearTimeout(t));
    exitTimers.forEach((t) => clearTimeout(t));
    detachDragListeners();
  });

  /// Flash any item whose text/priority changed (or that's brand new)
  /// between two list snapshots — used after voice edits land.
  function flashChangedRows(prev: NotchListDoc, next: NotchListDoc) {
    const byId = new Map(prev.items.map((i) => [i.id, i]));
    const changed = new Set<string>();
    for (const it of next.items) {
      if (it.done) continue;
      const before = byId.get(it.id);
      if (!before || before.text !== it.text || before.priority !== it.priority) {
        changed.add(it.id);
      }
    }
    if (changed.size === 0) return;
    setFlashIds(changed);
    if (flashTimer !== undefined) clearTimeout(flashTimer);
    flashTimer = setTimeout(() => {
      flashTimer = undefined;
      setFlashIds(new Set<string>());
    }, FLASH_MS);
  }

  /// Commit a completion to the backend. The list echo then prunes the id from
  /// the completion sets. On failure (rare — e.g. the item is already gone) the
  /// row is restored ONLY if the task genuinely still exists, so a commit that
  /// raced an echo-prune can't resurrect an already-removed row, and a real
  /// failure doesn't strand a task collapsed-and-invisible.
  function commitTask(id: string) {
    void notchListMarkDone(id).catch((e) => {
      console.error("notchListMarkDone:", e);
      if (doc().items.some((it) => it.id === id && !it.done)) {
        setExitingIds((s) => withId(s, id, false));
      }
    });
  }

  // --- Task completion (strike-through → undo → remove) -------------------
  // Ticking a task no longer removes it instantly. It enters a soft "limbo":
  // the row darkens, a line draws across it, and an Undo button is live for a
  // few seconds. We DEFER the backend `markDone` until the limbo resolves so
  // the row stays in the list and can animate in place. Resolutions:
  //   • Undo (button or re-tick the box)  → back to normal, nothing committed.
  //   • UNDO_MS elapses while hovering    → row collapses out, then commits.
  //   • Cursor leaves the notch           → commit at once (the panel is
  //                                          collapsing anyway, which is the
  //                                          smooth "it just happened" motion).
  // `completingIds` = struck + Undo live; `exitingIds` = past the window,
  // playing the collapse before commit. (The signals + timer maps are declared
  // up with the other state so the `headItem` memo can read `exitingIds`.)
  function clearTaskTimer(
    map: Map<string, ReturnType<typeof setTimeout>>,
    id: string,
  ) {
    const t = map.get(id);
    if (t !== undefined) {
      clearTimeout(t);
      map.delete(id);
    }
  }
  function withId(set: Set<string>, id: string, add: boolean): Set<string> {
    const next = new Set(set);
    if (add) next.add(id);
    else next.delete(id);
    return next;
  }

  /// Tick the checkbox: start (or, if already completing, undo) the limbo.
  /// All set mutations use functional updates so a concurrent echo-prune or a
  /// late markDone `.catch` can't clobber the latest state with a stale snapshot.
  function toggleComplete(id: string) {
    if (completingIds().has(id)) {
      cancelComplete(id);
      return;
    }
    if (exitingIds().has(id)) return; // already collapsing — past undo
    setCompletingIds((s) => withId(s, id, true));
    clearTaskTimer(undoTimers, id);
    undoTimers.set(id, setTimeout(() => finalizeComplete(id), UNDO_MS));
  }

  /// Undo: cancel the limbo entirely, leave the task active.
  function cancelComplete(id: string) {
    clearTaskTimer(undoTimers, id);
    clearTaskTimer(exitTimers, id);
    setCompletingIds((s) => withId(s, id, false));
    setExitingIds((s) => withId(s, id, false));
  }

  /// Window elapsed while still hovering → play the collapse-out, then commit.
  function finalizeComplete(id: string) {
    clearTaskTimer(undoTimers, id);
    setCompletingIds((s) => withId(s, id, false));
    setExitingIds((s) => withId(s, id, true));
    clearTaskTimer(exitTimers, id);
    exitTimers.set(
      id,
      setTimeout(() => {
        clearTaskTimer(exitTimers, id);
        // Commit, KEEPING the id in `exitingIds` (row stays collapsed) until the
        // echo prunes it — clearing it here would spring the row back to full
        // height for a frame before the backend removal lands.
        commitTask(id);
      }, EXIT_MS),
    );
  }

  /// Cursor left the notch — finalize every in-flight completion (the panel is
  /// gliding shut, so there's no per-row collapse). Called from `collapse()`.
  /// Only `completingIds` are committed here: ids already `exiting` own their
  /// commit via their own exit timer, so re-committing them would double-fire
  /// `markDone`. They stay marked `exiting` (excluded from the collapsed-bar
  /// head) until the echo prunes them.
  function commitPendingCompletions() {
    const completing = [...completingIds()];
    if (completing.length === 0) return;
    setExitingIds((s) => {
      const next = new Set(s);
      completing.forEach((id) => next.add(id));
      return next;
    });
    completing.forEach((id) => {
      clearTaskTimer(undoTimers, id);
      commitTask(id);
    });
    setCompletingIds(new Set<string>());
  }

  /// Drop completion bookkeeping for any task that has left the active list
  /// (committed `done`, or deleted) so the sets never leak stale ids. Driven by
  /// the `notch-list:changed` echo, this is also what finally clears a row out
  /// of `exitingIds` once its `markDone` lands. Functional updates + same-ref
  /// return (no change) keep it safe against interleaving mutations.
  function pruneCompletionState(next: NotchListDoc) {
    const active = new Set(next.items.filter((i) => !i.done).map((i) => i.id));
    const prune = (s: Set<string>): Set<string> => {
      let changed = false;
      const out = new Set<string>();
      s.forEach((id) => {
        if (active.has(id)) out.add(id);
        else {
          changed = true;
          clearTaskTimer(undoTimers, id);
          clearTaskTimer(exitTimers, id);
        }
      });
      return changed ? out : s;
    };
    setCompletingIds(prune);
    setExitingIds(prune);
  }

  /// True while a task is mid-completion — used to keep its row out of the
  /// drag gesture and to size the drawer as the row collapses.
  const pendingComplete = createMemo(
    () => new Set<string>([...completingIds(), ...exitingIds()]),
  );

  // --- Drag-to-reorder ----------------------------------------------------

  function detachDragListeners() {
    window.removeEventListener("pointermove", onDragMove);
    window.removeEventListener("pointerup", onDragEnd);
    window.removeEventListener("pointercancel", onDragEnd);
    stopAutoScroll();
  }

  /// CONTENT-space pointer Y for the current gesture: viewport-top-relative plus
  /// the live `scrollTop`, so the math is stable as the list scrolls under the
  /// pointer during a drag. Falls back to raw client Y if the list ref is gone.
  function pointerContentY(clientY: number): number {
    if (!listRef) return clientY;
    const r = listRef.getBoundingClientRect();
    return clientY - r.top + listRef.scrollTop;
  }

  function onHandlePointerDown(e: PointerEvent, index: number) {
    // Only in the expanded list; never in the collapsed single line. (Empty
    // lists expand to just the hint, which has no rows or drag handles.)
    if (!hoverExpanded()) return;
    // Don't start a drag while ANY row is mid-completion: a 0-height collapsing
    // row would throw off the row-index math (clamps, drop-line, sibling shifts).
    if (pendingComplete().size > 0) return;
    e.preventDefault();
    e.stopPropagation();
    dragStartY = e.clientY;
    dragLastClientY = e.clientY;
    dragStartContentY = pointerContentY(e.clientY);
    dragStartIndex = index;
    dragRowH = ROW_H;
    setDragId(rows()[index]?.id ?? null);
    setDragLifted(false);
    setDragOffsetY(0);
    setDropIndex(index);
    window.addEventListener("pointermove", onDragMove);
    window.addEventListener("pointerup", onDragEnd);
    window.addEventListener("pointercancel", onDragEnd);
  }

  function onDragMove(e: PointerEvent) {
    if (dragId() === null) return;
    if (!dragLifted()) {
      if (Math.abs(e.clientY - dragStartY) < DRAG_THRESHOLD) return;
      setDragLifted(true);
      startAutoScroll();
    }
    dragLastClientY = e.clientY;
    recomputeDrag();
  }

  /// Recompute the lifted row's offset + drop slot from the LAST pointer
  /// position, in content space (so it's correct mid-scroll). Shared by
  /// `onDragMove` and the auto-scroll ticker (which moves content with the
  /// pointer held still). X is locked — only Y matters.
  function recomputeDrag() {
    if (dragId() === null) return;
    const list = rows();
    const offset = pointerContentY(dragLastClientY) - dragStartContentY;
    // Clamp the lifted row to the list bounds.
    const maxUp = -dragStartIndex * dragRowH;
    const maxDown = (list.length - 1 - dragStartIndex) * dragRowH;
    const clamped = Math.max(maxUp, Math.min(maxDown, offset));
    setDragOffsetY(clamped);
    // Insertion index = origin shifted by whole-row steps.
    const steps = Math.round(clamped / dragRowH);
    const idx = Math.max(0, Math.min(list.length - 1, dragStartIndex + steps));
    setDropIndex(idx);
  }

  /// Edge auto-scroll: while a lifted row's pointer sits within DRAG_EDGE_PX of
  /// the scroll viewport's top/bottom, nudge `scrollTop` each frame so
  /// below-the-fold slots become reachable (the list is now taller than its
  /// viewport). Recomputes the drag after scrolling so the row + drop-line track
  /// the content. Self-cancels when the lift ends or there's nothing to scroll.
  function startAutoScroll() {
    if (autoScrollRaf !== null) return;
    const tick = () => {
      if (dragId() === null || !dragLifted() || !listRef) {
        autoScrollRaf = null;
        return;
      }
      const r = listRef.getBoundingClientRect();
      const maxScroll = listRef.scrollHeight - listRef.clientHeight;
      let next = listRef.scrollTop;
      if (dragLastClientY < r.top + DRAG_EDGE_PX) {
        next = Math.max(0, next - DRAG_SCROLL_STEP);
      } else if (dragLastClientY > r.bottom - DRAG_EDGE_PX) {
        next = Math.min(maxScroll, next + DRAG_SCROLL_STEP);
      }
      if (next !== listRef.scrollTop) {
        listRef.scrollTop = next;
        recomputeDrag();
      }
      autoScrollRaf = requestAnimationFrame(tick);
    };
    autoScrollRaf = requestAnimationFrame(tick);
  }

  function stopAutoScroll() {
    if (autoScrollRaf !== null) {
      cancelAnimationFrame(autoScrollRaf);
      autoScrollRaf = null;
    }
  }

  function onDragEnd() {
    detachDragListeners();
    const id = dragId();
    const lifted = dragLifted();
    const target = dropIndex();
    // Reset the lift state immediately; the row settles via CSS. Mark the
    // dropped row as "settling" for one settle window so its transform back
    // to the resting slot rides the overshoot ease (not the lift's 0ms).
    setDragId(null);
    setDragLifted(false);
    setDragOffsetY(0);
    setDropIndex(null);
    if (id !== null && lifted && !reducedMotion()) {
      setSettleId(id);
      if (settleTimer !== undefined) clearTimeout(settleTimer);
      settleTimer = setTimeout(() => {
        settleTimer = undefined;
        setSettleId(null);
      }, 240);
    }
    if (id === null || !lifted || target === null) return;

    // Commit to the previewed slot (the drop-line the user was aiming at). We
    // do NOT gate on the pointer being inside the list rect: the list is now a
    // bounded scroll viewport, so reaching a far slot legitimately parks the
    // pointer at the auto-scrolling edge (outside the clipped rect). `target` is
    // already clamped to a valid slot, and an unchanged slot no-ops below.
    const ids = rows().map((i) => i.id);
    const from = ids.indexOf(id);
    if (from === -1 || from === target) return;
    const reordered = ids.slice();
    reordered.splice(from, 1);
    reordered.splice(target, 0, id);
    // The whole active list is rendered now, so `reordered` already IS the full
    // active-id order — no below-the-fold tail to append.
    const fullOrder = reordered;

    // Cross-band stick: the list sorts by (priority desc, order asc) and the
    // backend re-sorts on the echo, so a flat `order` rewrite alone makes a
    // row dropped across a priority boundary snap back. Adopt the band of the
    // neighbor at the drop slot (the row above the insertion point, or the row
    // below when dropped at the very top) so the dragged row STICKS where it
    // landed. Compute against the NEW visual order (`reordered`).
    const byId = new Map(doc().items.map((it) => [it.id, it]));
    const draggedItem = byId.get(id);
    const neighborId =
      target > 0 ? reordered[target - 1] : reordered[target + 1];
    const neighbor = neighborId ? byId.get(neighborId) : undefined;
    const targetPriority = neighbor?.priority ?? draggedItem?.priority;
    const bandChanged =
      draggedItem !== undefined &&
      targetPriority !== undefined &&
      targetPriority !== draggedItem.priority;

    const reorder = () =>
      notchListReorder(fullOrder).catch((err) => {
        console.error("notchListReorder:", err);
        // Reconciliation flows from the `notch-list:changed` echo; on failure
        // the existing `doc()` stands.
        void refresh();
      });

    if (bandChanged) {
      // Re-priority FIRST (it re-normalizes order on the backend), THEN
      // reorder so the final flat order is written last. Two
      // `notch-list:changed` echoes may arrive; the reconcile re-reads the
      // doc each time, so both are safe.
      void notchListSetPriority(id, targetPriority!)
        .then(reorder)
        .catch((err) => {
          console.error("notchListSetPriority:", err);
          void refresh();
        });
    } else {
      void reorder();
    }
  }

  // --- Transitions --------------------------------------------------------

  const rowTransition = () =>
    reducedMotion() ? "none" : "background 120ms ease, opacity 120ms ease";
  const checkboxTransition = () =>
    reducedMotion() ? "none" : "border-color 120ms ease, background 120ms ease";

  return (
    // Stable outer wrapper — ALWAYS the same element so `panelRef` (and the
    // ResizeObserver bound to it in onMount) never goes stale. Transparent,
    // shrink-wraps to content. A flex column, centered: the ear row on top,
    // the drawer directly below the cutout. DOM mouse handlers are a SECONDARY
    // hover trigger; `notch:hover` is the source of truth.
    <div
      ref={panelRef}
      class="inline-flex flex-col items-center"
      aria-label={isEmpty() ? "Vox Notch — empty" : "Vox Notch"}
      onMouseEnter={() => armHoverExpand()}
      onMouseLeave={() => collapse()}
    >
      {/* ===== Ear row — ALWAYS visible, height = cutoutHeight() ===== */}
      <EarRow
        earWidth={earWidth()}
        cutoutWidth={cutoutWidth()}
        height={cutoutHeight()}
        hovered={hovered()}
        hasNotch={hasNotch()}
        drawerPresent={drawerPresent()}
        reduced={reducedMotion()}
      />

      {/* ===== Drawer — below the cutout ===== */}
      <Drawer
        // Voice
        phase={phase()}
        phaseMessage={phaseMessage()}
        levels={levels()}
        cutoutWidth={cutoutWidth()}
        cutoutHeight={cutoutHeight()}
        // List
        hoverExpanded={hoverExpanded()}
        head={headItem()}
        rows={rows()}
        topN={topN()}
        expandedWidth={expandedWidth()}
        bridgeW={bridgeW()}
        voiceHotkey={voiceHotkey()}
        flashIds={flashIds()}
        reduced={reducedMotion()}
        // Completion (strike-through → undo → remove). `exitingCount` counts
        // the exiting rows so the drawer height shrinks in step as they
        // collapse (capped to the viewport in the height memo).
        completingIds={completingIds()}
        exitingIds={exitingIds()}
        exitingCount={rows().filter((r) => exitingIds().has(r.id)).length}
        onToggleComplete={toggleComplete}
        onUndo={cancelComplete}
        // Quick-add composer
        composing={composing()}
        newText={newText()}
        newPriority={newPriority()}
        onStartCompose={startCompose}
        onComposeInput={setNewText}
        onComposePriority={setNewPriority}
        onComposeSubmit={submitCompose}
        onComposeCancel={() => endCompose(true)}
        onComposeBlur={onComposerBlur}
        setComposerInput={bindComposerInput}
        // Drag
        dragId={dragId()}
        dragLifted={dragLifted()}
        dragOffsetY={dragOffsetY()}
        dropIndex={dropIndex()}
        settleId={settleId()}
        rowH={ROW_H}
        // Refs / callbacks
        setListRef={(el) => (listRef = el)}
        onHandleDown={onHandlePointerDown}
        rowTransition={rowTransition()}
        checkboxTransition={checkboxTransition()}
      />
    </div>
  );
}

/* ===========================================================================
 * Ear row — [Vox ear][black spacer][Notch ear]. Width-pinned and centered at
 * all times so the spacer sits under the physical camera bump. Ears AND spacer
 * are pure #000 so the row reads as one continuous black bar flanking the
 * cutout (no wallpaper gaps); inner corners (touching the spacer) square, outer
 * corners rounded. Notchless: the two ears butt together into one pill with a
 * hairline divider between the words.
 * ======================================================================== */
function EarRow(props: {
  earWidth: number;
  cutoutWidth: number;
  height: number;
  hovered: boolean;
  hasNotch: boolean;
  drawerPresent: boolean;
  reduced: boolean;
}) {
  const wordOpacity = () => (props.hovered ? 0.95 : 0.62);
  /// Word size eases between the slim idle and full hovered size. The ear box
  /// width animates in lockstep (same duration/easing) so the label stays
  /// proportional and never overflows mid-transition.
  const wordSize = () => (props.hovered ? EAR_FONT_HOVER : EAR_FONT_IDLE);
  const wordTransition = () =>
    props.reduced
      ? "none"
      : `opacity 180ms ${EASE_IN}, font-size ${NOTCH_ANIM_MS}ms ${EASE_IN}`;
  /// Outer bottom-corner radius: square (0) when a drawer hangs below so the
  /// ear fuses into the connected slab; rounded into a standalone nub when the
  /// idle notch is empty (no drawer).
  const outerBottom = () => (props.drawerPresent ? 0 : 12);
  /// The ear box glides its width (idle nub ↔ full) and its bottom-corner
  /// rounding (nub ↔ fused slab) so the whole assembly grows fluidly.
  const earBoxTransition = () =>
    props.reduced
      ? "none"
      : `width ${NOTCH_ANIM_MS}ms ${EASE_IN}, border-radius ${NOTCH_ANIM_MS}ms ${EASE_IN}`;

  // Notchless: collapse the cutout spacer to 0 and fuse the ears into one
  // pill, rounding the inner corners and dropping a 1px divider between them.
  const showCutout = () => props.hasNotch;

  return (
    <div
      class="flex"
      style={{
        height: `${props.height}px`,
        // No explicit width — the row shrink-wraps its (CSS-transitioning) ear
        // boxes + fixed spacer so it tracks the animating width frame-by-frame
        // (the wrapper is centered, so it needs no pinned width).
      }}
    >
      {/* Left ear — "Vox" */}
      <div
        class="flex items-center justify-center"
        style={{
          width: `${props.earWidth}px`,
          height: "100%",
          background: "var(--color-notch-surface, #000)",
          // With-notch: inner (right, toward cutout) corners SQUARE so the ear
          // hugs the cutout wall; outer (left) corners rounded — bottom-outer
          // squares off when a drawer fuses below (the connected slab).
          // Notchless: round all four so the fused [Vox|Notch] reads as one
          // pill, with a hairline divider on the seam.
          "border-radius": showCutout()
            ? `6px 0 0 ${outerBottom()}px`
            : `8px 0 0 ${outerBottom()}px`,
          "border-right": showCutout()
            ? undefined
            : "1px solid rgba(255,255,255,0.1)",
          transition: earBoxTransition(),
        }}
      >
        <span
          style={{
            "font-size": `${wordSize()}px`,
            "font-weight": 600,
            "letter-spacing": "-0.01em",
            color: "#fff",
            opacity: `${wordOpacity()}`,
            transition: wordTransition(),
          }}
        >
          Vox
        </span>
      </div>

      {/* Central spacer over the camera bump — BLACK (not transparent), so the
          two ears read as one continuous black bar flanking the cutout instead
          of separate tabs with wallpaper showing through the gaps. The physical
          camera occupies the middle (no pixels there); the spacer's #000 only
          paints the slivers beside it, bridging each ear to the true-black
          bump. Width still tracks the bump so the ears never overlap the lens. */}
      <Show when={showCutout()}>
        <div
          aria-hidden="true"
          style={{
            width: `${props.cutoutWidth}px`,
            height: "100%",
            background: "var(--color-notch-surface, #000)",
          }}
        />
      </Show>

      {/* Right ear — "Notch" */}
      <div
        class="flex items-center justify-center"
        style={{
          width: `${props.earWidth}px`,
          height: "100%",
          background: "var(--color-notch-surface, #000)",
          // With-notch: inner (left, toward cutout) corners SQUARE; outer
          // (right) corners rounded — bottom-outer squares off when a drawer
          // fuses below. Notchless: round all four (the seam is drawn by the
          // left ear's right divider).
          "border-radius": showCutout()
            ? `0 6px ${outerBottom()}px 0`
            : `0 8px ${outerBottom()}px 0`,
          transition: earBoxTransition(),
        }}
      >
        <span
          style={{
            "font-size": `${wordSize()}px`,
            "font-weight": 600,
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
  );
}

/* ===========================================================================
 * Drawer — the variable region below the cutout. State machine:
 *   empty (idle)           → height 0 (nothing; ears are a standalone nub)
 *   collapsed (items, idle)→ one slim line, full slab width: dimmed #1 task
 *   list (items, hover)    → wide; top-N rows (drag handle + checkbox + dot + text) + hint footer
 *   hint (empty, hover)    → compact ⌘⌥L keycap affordance, full slab width
 *   voice: listening       → waveform ribbon (full slab width)
 *   voice: compiling       → pulsing dots (full slab width)
 *   voice: error           → message text (briefly), then idle
 *
 * Connected slab: every state except the wide hovered `list` spans the full
 * ear-row width (`bridgeW`) so the drawer fuses with the ears into one black
 * panel rather than a narrow tab under the bump.
 *
 * Fluid size: the box width + height CSS-transition between states and the
 * outer ResizeObserver chases the live rect (`syncWindowSize`), so the OS
 * window glides with the box instead of snapping. `overflow-hidden` turns a
 * height change into a content wipe.
 * ======================================================================== */
function Drawer(props: {
  phase: DrawerPhase | null;
  phaseMessage: string;
  levels: number[];
  cutoutWidth: number;
  cutoutHeight: number;
  hoverExpanded: boolean;
  head: NotchItem | null;
  rows: NotchItem[];
  topN: number;
  expandedWidth: number;
  bridgeW: number;
  voiceHotkey: string;
  flashIds: Set<string>;
  reduced: boolean;
  completingIds: Set<string>;
  exitingIds: Set<string>;
  exitingCount: number;
  onToggleComplete: (id: string) => void;
  onUndo: (id: string) => void;
  // Quick-add composer
  composing: boolean;
  newText: string;
  newPriority: NotchPriority;
  onStartCompose: () => void;
  onComposeInput: (text: string) => void;
  onComposePriority: (p: NotchPriority) => void;
  onComposeSubmit: () => void;
  onComposeCancel: () => void;
  onComposeBlur: () => void;
  setComposerInput: (el: HTMLInputElement) => void;
  dragId: string | null;
  dragLifted: boolean;
  dragOffsetY: number;
  dropIndex: number | null;
  settleId: string | null;
  rowH: number;
  setListRef: (el: HTMLDivElement) => void;
  onHandleDown: (e: PointerEvent, index: number) => void;
  rowTransition: string;
  checkboxTransition: string;
}) {
  // What the drawer should render right now (priority: voice > hover > list).
  // On hover: the task LIST when there are items, or the compact add-task HINT
  // when the list is empty. Off hover: the #1-task line when there are items,
  // nothing when empty.
  const mode = createMemo<
    "voice" | "list" | "hint" | "collapsed" | "empty"
  >(() => {
    if (props.phase !== null) return "voice";
    if (props.hoverExpanded) return props.rows.length > 0 ? "list" : "hint";
    return props.rows.length === 0 ? "empty" : "collapsed";
  });

  // Height of the footer beneath the task rows: the typed-task composer when
  // open, otherwise the voice add-task hint.
  const footerH = () => (props.composing ? COMPOSE_H : HINT_H);

  // Target content height (px) for the current mode. 0 collapses the window.
  const targetHeight = createMemo(() => {
    switch (mode()) {
      case "voice":
        // Waveform / dots / error strip — short calm ribbon.
        return Math.round(props.cutoutHeight * 0.65) + 10;
      case "list": {
        // The list is a bounded, scrollable viewport: it shows at most `topN`
        // rows, then scrolls. Rows mid-collapse (a finalized completion) are
        // subtracted so the drawer shrinks in step as they collapse, but the
        // viewport never exceeds `topN` rows even with a long list.
        const visibleRows = Math.min(
          Math.max(0, props.rows.length - props.exitingCount),
          props.topN,
        );
        return visibleRows * props.rowH + LIST_PAD_Y * 2 + footerH();
      }
      case "hint":
        // Empty + hovered: the compact keycap affordance — or, while composing,
        // the typed-task input standing alone.
        return props.composing ? COMPOSE_H : EMPTY_HINT_H;
      case "collapsed":
        return COLLAPSED_H;
      default:
        return 0;
    }
  });

  // Target content width. Collapsed task / empty-hover hint / voice strip all
  // span the full ear-row width (`bridgeW`) so they fuse into one connected
  // black slab under the ears. The hovered task LIST — and the composer in any
  // mode (so the input has room to type) — fan out wider.
  const targetWidth = createMemo(() => {
    if (mode() === "list" || props.composing) return props.expandedWidth;
    return props.bridgeW;
  });

  return (
    <div
      class="overflow-hidden"
      style={{
        // Fluid size: width + height CSS-transition to their target, and the
        // outer ResizeObserver chases the live rect so the OS window glides in
        // lockstep (replaces the old instant-snap "single-resize" discipline).
        // `overflow-hidden` makes the height transition double as a content
        // wipe. Background stays solid black always — at height 0 the box is
        // invisible anyway, and keeping it black avoids a flash to transparent
        // while a collapse animates down.
        height: `${targetHeight()}px`,
        width: `${targetWidth()}px`,
        transition: props.reduced
          ? "none"
          : `height ${NOTCH_ANIM_MS}ms ${EASE_IN}, width ${NOTCH_ANIM_MS}ms ${EASE_IN}`,
        background: "var(--color-notch-surface, #000)",
        // Top corners SQUARE (fused under the cutout); bottom rounded.
        "border-radius": "0 0 16px 16px",
        color: "var(--color-ink)",
      }}
    >
      <Show when={mode() === "voice"}>
        <VoiceStrip
          phase={props.phase}
          message={props.phaseMessage}
          levels={props.levels}
          cutoutHeight={props.cutoutHeight}
          reduced={props.reduced}
        />
      </Show>

      <Show when={mode() === "collapsed"}>
        <Show when={props.head}>
          {(item) => (
            // Idle glance: the single top task, DIMMED (muted-grey text + a
            // desaturated priority dot) and small (8px), centered both axes.
            // The full-width slab keeps the pair centered under the camera; the
            // inner wrapper is capped to the cutout zone so a long task
            // truncates under the bump rather than sprawling beneath the ears.
            // Hovering swaps to the bright, full-size left-aligned ExpandedList.
            <div
              class="flex w-full items-center justify-center"
              classList={{ "notch-flash": props.flashIds.has(item().id) }}
              style={{ height: `${COLLAPSED_H}px` }}
            >
              <div
                class="flex min-w-0 items-center gap-1.5"
                style={{ "max-width": `${props.cutoutWidth}px` }}
              >
                <span
                  class="h-[4px] w-[4px] shrink-0 rounded-full"
                  style={{ background: priorityColorDim(item().priority) }}
                  aria-hidden="true"
                />
                <span class="min-w-0 truncate text-[8px] leading-[12px] text-[var(--color-ink-mute)]">
                  {item().text}
                </span>
              </div>
            </div>
          )}
        </Show>
      </Show>

      {/* Hover-expanded content (the "list" and "hint" modes share one block).
          The task list mounts only when there are rows; the footer is ONE
          element — composer or add-hint. Keeping the composer in a single
          `<Show when={composing}>` (NOT duplicated per mode) means adding the
          first task, which flips empty→list, never unmounts/remounts the
          focused <input> — so a remove-from-DOM blur can't slam the composer
          shut mid-multi-add. */}
      <Show when={mode() === "list" || mode() === "hint"}>
        <Show when={props.rows.length > 0}>
          <ExpandedList
            rows={props.rows}
            maxRows={props.topN}
            flashIds={props.flashIds}
            reduced={props.reduced}
            completingIds={props.completingIds}
            exitingIds={props.exitingIds}
            onToggleComplete={props.onToggleComplete}
            onUndo={props.onUndo}
            dragId={props.dragId}
            dragLifted={props.dragLifted}
            dragOffsetY={props.dragOffsetY}
            dropIndex={props.dropIndex}
            settleId={props.settleId}
            rowH={props.rowH}
            setListRef={props.setListRef}
            onHandleDown={props.onHandleDown}
            rowTransition={props.rowTransition}
            checkboxTransition={props.checkboxTransition}
          />
        </Show>
        <Show
          when={props.composing}
          fallback={
            <Show
              when={props.rows.length > 0}
              fallback={
                <AddHintCompact
                  hotkey={props.voiceHotkey}
                  onAdd={props.onStartCompose}
                />
              }
            >
              <AddHint
                hotkey={props.voiceHotkey}
                onAdd={props.onStartCompose}
              />
            </Show>
          }
        >
          <AddComposer
            text={props.newText}
            priority={props.newPriority}
            onInput={props.onComposeInput}
            onPriority={props.onComposePriority}
            onSubmit={props.onComposeSubmit}
            onCancel={props.onComposeCancel}
            onBlur={props.onComposeBlur}
            setInputRef={props.setComposerInput}
          />
        </Show>
      </Show>
    </div>
  );
}

/// Plus glyph for the click-to-add affordances. `currentColor` so it inherits
/// the surrounding text color (which brightens on hover via CSS).
function PlusGlyph(props: { size?: number }) {
  const s = () => props.size ?? 10;
  return (
    <svg
      width={s()}
      height={s()}
      viewBox="0 0 12 12"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M6 2.4V9.6M2.4 6H9.6"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
      />
    </svg>
  );
}

/* ===========================================================================
 * Add-task footer (non-empty list) — a CLICK-to-type "＋ Add a task" button on
 * the left plus the notch voice hotkey (default ⌥⌘L) as keycaps on the right.
 * Two ways to add a task without leaving the notch: click the CTA to type, or
 * press the shown shortcut to dictate. Clicking the CTA opens the composer.
 * ======================================================================== */
function AddHint(props: { hotkey: string; onAdd: () => void }) {
  const tokens = createMemo(() => formatHotkeyTokens(props.hotkey));
  return (
    <div
      class="flex items-center justify-between gap-2 px-2.5"
      style={{ height: `${HINT_H}px` }}
    >
      <button
        type="button"
        class="notch-add-cta"
        onClick={props.onAdd}
        aria-label="Add a task"
      >
        <PlusGlyph />
        <span>Add a task</span>
      </button>
      <span
        class="flex items-center gap-1"
        aria-hidden="true"
        title="…or add by voice"
      >
        <For each={tokens()}>
          {(t) => <kbd class="notch-kbd">{t.symbol}</kbd>}
        </For>
      </span>
    </div>
  );
}

/* ===========================================================================
 * Compact add-task footer — the EMPTY + hovered drawer content. The whole row
 * is one click-to-type "＋ Add a task" button (with the tiny ⌥⌘L keys trailing
 * as the voice hint), shrunk to EMPTY_HINT_H so an empty hovered notch invites
 * a first task without hanging over app chrome below the menu bar.
 * ======================================================================== */
function AddHintCompact(props: { hotkey: string; onAdd: () => void }) {
  const tokens = createMemo(() => formatHotkeyTokens(props.hotkey));
  return (
    <button
      type="button"
      class="notch-add-cta-compact"
      style={{ height: `${EMPTY_HINT_H}px` }}
      onClick={props.onAdd}
      aria-label="Add a task"
    >
      <PlusGlyph size={8} />
      <span class="text-[9px] leading-none">Add a task</span>
      <span class="flex items-center gap-0.5" aria-hidden="true">
        <For each={tokens()}>
          {(t) => <kbd class="notch-kbd-sm">{t.symbol}</kbd>}
        </For>
      </span>
    </button>
  );
}

/* ===========================================================================
 * Quick-add composer — the typed-task input that replaces the add-task footer
 * while the user is composing. A 3-dot priority picker on the left + a text
 * input. Enter adds the task and keeps the input open for rapid multi-add;
 * Escape closes it. The priority dots `preventDefault` on mousedown so clicking
 * one never blurs (and closes) the input.
 * ======================================================================== */
function AddComposer(props: {
  text: string;
  priority: NotchPriority;
  onInput: (t: string) => void;
  onPriority: (p: NotchPriority) => void;
  onSubmit: () => void;
  onCancel: () => void;
  onBlur: () => void;
  setInputRef: (el: HTMLInputElement) => void;
}) {
  const priorities: NotchPriority[] = ["high", "med", "low"];
  return (
    <div
      class="flex items-center gap-2 px-2.5"
      style={{ height: `${COMPOSE_H}px` }}
    >
      <div class="flex items-center gap-1" role="group" aria-label="Priority">
        <For each={priorities}>
          {(p) => (
            <button
              type="button"
              class="notch-prio-dot"
              classList={{ "is-active": props.priority === p }}
              style={{ "--dot": priorityColor(p) }}
              aria-label={`${p} priority`}
              aria-pressed={props.priority === p}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => props.onPriority(p)}
            />
          )}
        </For>
      </div>
      <input
        ref={props.setInputRef}
        type="text"
        class="notch-add-input"
        placeholder="Add a task…"
        value={props.text}
        spellcheck={false}
        autocomplete="off"
        onInput={(e) => props.onInput(e.currentTarget.value)}
        onBlur={props.onBlur}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            props.onSubmit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            props.onCancel();
          }
          e.stopPropagation();
        }}
      />
    </div>
  );
}

/* ===========================================================================
 * Voice strip — waveform (listening) / dots (compiling) / message (error).
 * Calm muted-white acknowledgment ribbon, NOT the pill's coral.
 * ======================================================================== */
function VoiceStrip(props: {
  phase: DrawerPhase | null;
  message: string;
  levels: number[];
  cutoutHeight: number;
  reduced: boolean;
}) {
  // Waveform bars cap LOW (~0.5× the band) so it reads as a quiet ribbon.
  const maxBar = () => Math.max(10, Math.round(props.cutoutHeight * 0.5));
  const stripH = () => Math.round(props.cutoutHeight * 0.65);

  return (
    <div
      class="flex w-full items-center justify-center px-3"
      style={{ height: "100%" }}
    >
      <Show when={props.phase === "listening"}>
        <div
          class="flex w-full items-center justify-between"
          style={{ height: `${stripH()}px` }}
          aria-hidden="true"
        >
          {/* `Index` (not `For`): the levels array is a fixed-length window
              whose VALUES change every audio tick. `For` keys by value and
              freezes each bar's amplitude at creation (the bars stayed black
              while speaking); `Index` keys by position and hands each bar a
              `level()` SIGNAL, so heights update in place and the CSS height
              transition animates a live equalizer. */}
          <Index each={props.levels}>
            {(level, i) => {
              // Sine bell — dissolve into the black at both edges. Static: it
              // depends only on the bar's fixed index + window length.
              const env = edgeEnvelope(i, props.levels.length);
              // Gamma 0.65 opens the bottom of the range (RMS lives low).
              // Reactive via the `level()` signal so the bar tracks live RMS.
              const shaped = () => Math.pow(Math.max(0, Math.min(1, level())), 0.65);
              const h = () =>
                Math.max(
                  1.5,
                  Math.round(
                    (0.22 + shaped() * 0.78) * maxBar() * (0.4 + 0.6 * env),
                  ),
                );
              const opacity = () => (0.32 + shaped() * 0.68) * env;
              return (
                <div
                  style={{
                    width: "1.5px",
                    height: `${h()}px`,
                    "border-radius": "1.5px",
                    // Cool silver shimmer — calm acknowledgment, not pill coral.
                    background: WAVE_COLOR,
                    opacity: opacity(),
                    transition: props.reduced
                      ? "none"
                      : "height 110ms ease-out, opacity 180ms ease-out",
                  }}
                />
              );
            }}
          </Index>
        </div>
      </Show>

      <Show when={props.phase === "compiling"}>
        <div class="flex items-center gap-1.5" aria-label="Working">
          <span class="notch-dot" />
          <span class="notch-dot" style={{ "animation-delay": "0.15s" }} />
          <span class="notch-dot" style={{ "animation-delay": "0.3s" }} />
        </div>
      </Show>

      <Show when={props.phase === "error"}>
        <span class="truncate text-[11px] leading-[14px] text-[var(--color-ink-mute,rgba(255,255,255,.62))]">
          {props.message || "Couldn't understand that"}
        </span>
      </Show>
    </div>
  );
}

/* ===========================================================================
 * Expanded list — every active row (drag handle + checkbox + priority dot +
 * text), in a bounded VIEWPORT that shows up to `maxRows` and SCROLLS the rest
 * into view, so a long list is fully reachable rather than clipped at the fold.
 * Rows fade+rise in on mount with a ~20ms stagger. Drag lifts a row to follow
 * the pointer; siblings slide; a 2px accent drop-line marks the insertion point.
 * ======================================================================== */
function ExpandedList(props: {
  rows: NotchItem[];
  maxRows: number;
  flashIds: Set<string>;
  reduced: boolean;
  completingIds: Set<string>;
  exitingIds: Set<string>;
  onToggleComplete: (id: string) => void;
  onUndo: (id: string) => void;
  dragId: string | null;
  dragLifted: boolean;
  dragOffsetY: number;
  dropIndex: number | null;
  settleId: string | null;
  rowH: number;
  setListRef: (el: HTMLDivElement) => void;
  onHandleDown: (e: PointerEvent, index: number) => void;
  rowTransition: string;
  checkboxTransition: string;
}) {
  const draggingActive = () =>
    props.dragId !== null && props.dragLifted;

  // `<For>` is reference-keyed and every list echo hands it fresh item objects,
  // so it re-mounts every row — which would replay the fade-rise entrance on
  // untouched rows after each voice edit / completion. Track which ids have
  // already entered (per open session — this whole list unmounts on collapse)
  // so the entrance only plays the first time a row appears.
  const seenRowIds = new Set<string>();

  // The slot index the dragged row currently occupies (its origin).
  const dragOriginIndex = () =>
    props.dragId === null
      ? -1
      : props.rows.findIndex((r) => r.id === props.dragId);

  // Per-row sibling shift while dragging: rows between the origin and the
  // drop target slide by one row height to open the gap.
  function siblingShift(index: number): number {
    if (!draggingActive()) return 0;
    const from = dragOriginIndex();
    const to = props.dropIndex;
    if (from === -1 || to === null || index === from) return 0;
    if (from < to) {
      // Dragging down: rows in (from, to] slide UP.
      if (index > from && index <= to) return -props.rowH;
    } else if (from > to) {
      // Dragging up: rows in [to, from) slide DOWN.
      if (index >= to && index < from) return props.rowH;
    }
    return 0;
  }

  return (
    <div
      ref={props.setListRef}
      class="notch-scroll relative flex flex-col px-2"
      style={{
        "padding-top": `${LIST_PAD_Y}px`,
        "padding-bottom": `${LIST_PAD_Y}px`,
        // Bounded viewport: show up to `maxRows`, then scroll the rest into
        // view. `overflow-x: hidden` keeps the lifted-row shadow from spawning a
        // horizontal scrollbar; the drop-line + row transforms live in the
        // scrolled content box, so they track the rows as the user scrolls.
        "max-height": `${props.maxRows * props.rowH + LIST_PAD_Y * 2}px`,
        "overflow-y": "auto",
        "overflow-x": "hidden",
      }}
    >
      {/* Accent drop-line at the insertion point (drag only). */}
      <Show when={draggingActive() && props.dropIndex !== null}>
        <div
          class="pointer-events-none absolute inset-x-2"
          style={{
            top: `${LIST_PAD_Y + (props.dropIndex ?? 0) * props.rowH - 1}px`,
            height: "2px",
            background: "var(--color-accent, #3fb8b0)",
            "border-radius": "1px",
          }}
          aria-hidden="true"
        />
      </Show>

      <For each={props.rows}>
        {(item, i) => {
          const isDragged = () => props.dragId === item.id && props.dragLifted;
          const enterDelay = () =>
            props.reduced ? "0ms" : `${i() * 20}ms`;
          const liftTransform = () => {
            if (isDragged()) {
              return `translateY(${props.dragOffsetY}px) scale(1.02)`;
            }
            const shift = siblingShift(i());
            return shift ? `translateY(${shift}px)` : "translateY(0)";
          };
          const dragTransition = () => {
            if (props.reduced) return "none";
            if (isDragged()) {
              // While following the pointer: no transform transition — we set
              // the transform on every move, so any easing would lag the
              // cursor.
              return "transform 0ms";
            }
            // The just-dropped row settles back to its slot with a slight
            // overshoot; ordinary siblings just slide. The background/shadow
            // (lift wash) rides the shared row transition either way.
            const t =
              props.settleId === item.id
                ? `transform 220ms ${EASE_SETTLE}`
                : `transform 160ms ${EASE_IN}`;
            return `${t}, ${props.rowTransition}, box-shadow 120ms ease`;
          };
          // Completion limbo: `completing` = struck + Undo live; `exiting` =
          // past the window, collapsing out before commit. Both DIM the row.
          const completing = () => props.completingIds.has(item.id);
          const exiting = () => props.exitingIds.has(item.id);
          const dimmed = () => completing() || exiting();
          // Only the FIRST appearance of an id plays the entrance (see
          // `seenRowIds`); re-mounts from later echoes skip it.
          const firstSeen = !seenRowIds.has(item.id);
          onMount(() => seenRowIds.add(item.id));
          return (
            <div
              class="flex items-center gap-2 rounded-[8px] px-1.5 hover:bg-white/[0.06]"
              classList={{
                "notch-flash": props.flashIds.has(item.id),
                // Mount-reveal: fade + rise with a per-row stagger. Disabled
                // for any row once a drag is in flight (the lift/shift
                // transform would fight the keyframe's transform), while a row
                // is flashing, and while it's mid-completion (its own
                // strike/collapse owns the motion).
                "notch-row-enter":
                  firstSeen &&
                  !draggingActive() &&
                  !props.reduced &&
                  !dimmed() &&
                  !props.flashIds.has(item.id),
              }}
              style={{
                // Exiting rows collapse to 0 height + fade — the list above
                // reflows up and the drawer shrinks in step (see exitingCount).
                height: exiting() ? "0px" : `${props.rowH}px`,
                "min-height": "0",
                overflow: exiting() ? "hidden" : undefined,
                "z-index": isDragged() ? 5 : 1,
                position: "relative",
                transform: liftTransform(),
                transition: exiting()
                  ? `height ${EXIT_MS}ms ${EASE_IN}, opacity ${EXIT_MS}ms ${EASE_IN}`
                  : dragTransition(),
                background: isDragged() ? "rgba(255,255,255,0.10)" : undefined,
                "box-shadow": isDragged()
                  ? "0 6px 16px rgba(0,0,0,0.55)"
                  : "none",
                opacity: exiting() ? 0 : 1,
                "pointer-events": exiting() ? "none" : undefined,
                "animation-delay": draggingActive() ? "0ms" : enterDelay(),
              }}
            >
              {/* Drag handle — fades out once a row is mid-completion (it can't
                  be dragged then). */}
              <div
                class="notch-drag-handle shrink-0"
                style={{
                  "touch-action": "none",
                  opacity: dimmed() ? "0" : undefined,
                  transition: "opacity 160ms ease",
                }}
                onPointerDown={(e) => props.onHandleDown(e, i())}
                aria-hidden="true"
              >
                <span class="notch-drag-dot" />
                <span class="notch-drag-dot" />
                <span class="notch-drag-dot" />
                <span class="notch-drag-dot" />
                <span class="notch-drag-dot" />
                <span class="notch-drag-dot" />
              </div>

              {/* Checkbox — tick starts the completion; re-ticking it undoes.
                  Fills accent-teal with a check while completing. */}
              <button
                type="button"
                aria-label={dimmed() ? "Undo complete" : "Mark done"}
                onClick={(e) => {
                  e.stopPropagation();
                  props.onToggleComplete(item.id);
                }}
                class="flex h-[15px] w-[15px] shrink-0 items-center justify-center rounded-full border"
                style={{
                  "border-color": dimmed()
                    ? "var(--color-accent, #3fb8b0)"
                    : CHECKBOX_BORDER,
                  background: dimmed()
                    ? "var(--color-accent, #3fb8b0)"
                    : "transparent",
                  transition: props.checkboxTransition,
                }}
              >
                <Show when={dimmed()}>
                  <svg
                    width="9"
                    height="9"
                    viewBox="0 0 12 12"
                    fill="none"
                    aria-hidden="true"
                  >
                    <path
                      d="M2.5 6.4 L5 8.8 L9.5 3.6"
                      stroke="#06231f"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </Show>
              </button>

              <span
                class="h-[7px] w-[7px] shrink-0 rounded-full"
                style={{
                  background: priorityColor(item.priority),
                  opacity: dimmed() ? "0.4" : "1",
                  transition: "opacity 200ms ease",
                }}
                aria-hidden="true"
              />

              {/* Task text with the strike-through overlay. The span shrink-
                  wraps the text (truncating when long) so the line matches the
                  text width; it draws across over STRIKE_MS, retracts fast on
                  undo. */}
              <span
                class="relative min-w-0 truncate text-[12px] leading-[16px]"
                style={{
                  color: "var(--color-ink)",
                  opacity: dimmed() ? "0.45" : "1",
                  transition: "opacity 200ms ease",
                }}
              >
                {item.text}
                <span
                  class="notch-strike-line"
                  style={{
                    transform: dimmed() ? "scaleX(1)" : "scaleX(0)",
                    transition: props.reduced
                      ? "none"
                      : `transform ${dimmed() ? STRIKE_MS : 220}ms ${
                          dimmed() ? "linear" : EASE_IN
                        }`,
                  }}
                  aria-hidden="true"
                />
              </span>

              {/* Undo — live during the window, gone once the row collapses. */}
              <Show when={completing()}>
                <button
                  type="button"
                  class="notch-undo ml-auto shrink-0"
                  onClick={(e) => {
                    e.stopPropagation();
                    props.onUndo(item.id);
                  }}
                >
                  Undo
                </button>
              </Show>
            </div>
          );
        }}
      </For>
    </div>
  );
}
