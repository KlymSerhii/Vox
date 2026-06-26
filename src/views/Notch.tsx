import {
  createEffect,
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
  notchListMarkDone,
  notchListReorder,
  notchListSetPriority,
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
} from "../lib/ipc";
import { formatHotkey, formatHotkeyTokens } from "../lib/hotkey";

/// Default top-N if settings haven't loaded yet (matches the backend
/// `default_notch_list_top_n`).
const DEFAULT_TOP_N = 5;
/// Default hover intent delay (matches `default_notch_list_hover_delay_ms`).
const DEFAULT_HOVER_DELAY_MS = 120;

/// Expanded-row height (drag handle + checkbox + priority dot + text).
const ROW_H = 28;
/// Vertical padding inside the expanded list (top + bottom).
const LIST_PAD_Y = 6;
/// Collapsed single-line drawer height (the #1-task bar). Kept tight (a 12px
/// line at leading-16 exactly fills it, no extra padding) so the idle nub
/// hangs as little as possible below the camera bump — at the old 30px it
/// overlapped app chrome right under the menu bar (e.g. browser tabs).
const COLLAPSED_H = 16;
/// Add-task hint row height — the keycap affordance revealed on hover. It's
/// the only drawer content when the list is empty, and a faint footer beneath
/// the rows when it isn't.
const HINT_H = 30;

/* ---------------------------------------------------------------------------
 * Ear-row + cutout geometry — logical px. The window is TOP-ANCHORED at the
 * screen's top edge (Rust owns the y-anchor + re-centering). The view is one
 * vertical stack:
 *
 *   [ear-row]   <- always visible: [Vox ear][black spacer][Notch ear]
 *   [drawer]    <- below the cutout: 0-height when idle/empty, grows on hover
 *
 * The ear row is width-pinned (earW*2 + cutoutWidth) and centered so the
 * black spacer always sits under the physical camera bump. Only the drawer
 * changes width.
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

/// Ear width clamp + bounds. Sized to seat the longer "Notch" label with
/// comfortable horizontal padding at the word size below, tied to the cutout
/// so the assembly stays proportional across notch geometries.
const EAR_W_MIN = 46;
const EAR_W_MAX = 56;

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
/// How long the reserved drawer height is held after collapse so the exit
/// animation can play before the single resize-down. Slightly over the
/// longest exit transition.
const RETRACT_MS = 210;
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
  /// The notch voice hotkey (default ⌥⌘L), shown in the on-hover add-task
  /// hint. Kept in sync via `applySettings` / `settings:changed`.
  const [voiceHotkey, setVoiceHotkey] = createSignal<string>("Cmd+Alt+L");
  /// Physical-notch geometry, pushed from Rust.
  const [geo, setGeo] = createSignal<NotchGeometry>({
    has_notch: false,
    width: 0,
    height: 0,
  });

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
    setHovered(false);
    queueMicrotask(syncWindowSize);
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

  const top = createMemo(() => activeItems().slice(0, topN()));
  const isEmpty = createMemo(() => activeItems().length === 0);
  const headItem = createMemo(() => activeItems()[0] ?? null);

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

  /// Ear width — tied to the cutout (~0.24×), clamped to
  /// [EAR_W_MIN, EAR_W_MAX] so the "Notch" label keeps padding on both edges.
  const earWidth = createMemo(() =>
    Math.round(
      Math.min(EAR_W_MAX, Math.max(EAR_W_MIN, cutoutWidth() * 0.24)),
    ),
  );

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

  /// Drawer shows the voice strip whenever a phase is active.
  const voiceActive = createMemo(() => phase() !== null);
  /// Hover-expanded drawer: any time the notch is hovered and not mid-voice.
  /// Drives both the task list (when there are items) and the add-task hint
  /// (always — the hint is the sole drawer content when the list is empty).
  const hoverExpanded = createMemo(() => hovered() && !voiceActive());

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
  /// and snaps back on leave. `notch_resize` re-centers the window below the
  /// notch (the frontend can't know the display origin), so it owns sizing.
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
    unlistenList = await onNotchListChanged((next) => {
      flashChangedRows(doc(), next);
      setDoc(next);
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
    unlistenHover?.();
    unlistenListening?.();
    unlistenLevels?.();
    ro?.disconnect();
    clearHoverTimer();
    clearErrorTimer();
    if (flashTimer !== undefined) clearTimeout(flashTimer);
    if (settleTimer !== undefined) clearTimeout(settleTimer);
    if (levelsDecayTimer !== undefined) clearInterval(levelsDecayTimer);
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

  async function markDone(id: string) {
    try {
      await notchListMarkDone(id);
    } catch (e) {
      console.error("notchListMarkDone:", e);
    }
  }

  // --- Drag-to-reorder ----------------------------------------------------

  function detachDragListeners() {
    window.removeEventListener("pointermove", onDragMove);
    window.removeEventListener("pointerup", onDragEnd);
    window.removeEventListener("pointercancel", onDragEnd);
  }

  function onHandlePointerDown(e: PointerEvent, index: number) {
    // Only in the expanded list; never in the collapsed single line. (Empty
    // lists expand to just the hint, which has no rows or drag handles.)
    if (!hoverExpanded()) return;
    e.preventDefault();
    e.stopPropagation();
    dragStartY = e.clientY;
    dragStartIndex = index;
    dragRowH = ROW_H;
    setDragId(top()[index]?.id ?? null);
    setDragLifted(false);
    setDragOffsetY(0);
    setDropIndex(index);
    window.addEventListener("pointermove", onDragMove);
    window.addEventListener("pointerup", onDragEnd);
    window.addEventListener("pointercancel", onDragEnd);
  }

  function onDragMove(e: PointerEvent) {
    if (dragId() === null) return;
    const dy = e.clientY - dragStartY;
    if (!dragLifted()) {
      if (Math.abs(dy) < DRAG_THRESHOLD) return;
      setDragLifted(true);
    }
    const rows = top();
    // Clamp the lifted row to the list bounds (Y only — X is locked).
    const maxUp = -dragStartIndex * dragRowH;
    const maxDown = (rows.length - 1 - dragStartIndex) * dragRowH;
    const clamped = Math.max(maxUp, Math.min(maxDown, dy));
    setDragOffsetY(clamped);
    // Insertion index = origin shifted by whole-row steps.
    const steps = Math.round(clamped / dragRowH);
    const idx = Math.max(0, Math.min(rows.length - 1, dragStartIndex + steps));
    setDropIndex(idx);
  }

  function onDragEnd(e: PointerEvent) {
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
    if (id === null || !lifted) return;

    // Released outside the drawer → spring back, no reorder.
    const within = listRef && pointerWithin(listRef, e.clientX, e.clientY);
    if (!within || target === null) return;

    const rows = top().map((i) => i.id);
    const from = rows.indexOf(id);
    if (from === -1 || from === target) return;
    const reordered = rows.slice();
    reordered.splice(from, 1);
    reordered.splice(target, 0, id);
    // Full active-id order: reordered top-N, then the remaining tail in its
    // existing relative order, so items below the fold keep their positions.
    const tail = activeItems()
      .map((i) => i.id)
      .filter((x) => !rows.includes(x));
    const fullOrder = [...reordered, ...tail];

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

  function pointerWithin(el: HTMLElement, x: number, y: number): boolean {
    const r = el.getBoundingClientRect();
    // Generous horizontal slop (the lifted row may overhang) but require
    // the pointer to stay within the vertical band.
    return y >= r.top - 8 && y <= r.bottom + 8 && x >= r.left - 40 && x <= r.right + 40;
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
        rows={top()}
        expandedWidth={expandedWidth()}
        voiceHotkey={voiceHotkey()}
        flashIds={flashIds()}
        reduced={reducedMotion()}
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
        onMarkDone={markDone}
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
  reduced: boolean;
}) {
  const wordOpacity = () => (props.hovered ? 0.95 : 0.62);
  const wordTransition = () =>
    props.reduced ? "opacity 100ms linear" : `opacity 180ms ${EASE_IN}`;

  // Notchless: collapse the cutout spacer to 0 and fuse the ears into one
  // pill, rounding the inner corners and dropping a 1px divider between them.
  const showCutout = () => props.hasNotch;

  return (
    <div
      class="flex"
      style={{
        height: `${props.height}px`,
        width: `${props.earWidth * 2 + (showCutout() ? props.cutoutWidth : 0)}px`,
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
          // hugs the cutout wall; outer (left) corners rounded.
          // Notchless: round all four so the fused [Vox|Notch] reads as one
          // pill, with a hairline divider on the seam.
          "border-radius": showCutout() ? "6px 0 0 12px" : "8px 0 0 12px",
          "border-right": showCutout()
            ? undefined
            : "1px solid rgba(255,255,255,0.1)",
        }}
      >
        <span
          style={{
            "font-size": "11px",
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
          // (right) corners rounded. Notchless: round all four (the seam is
          // drawn by the left ear's right divider).
          "border-radius": showCutout() ? "0 6px 12px 0" : "0 8px 12px 0",
        }}
      >
        <span
          style={{
            "font-size": "11px",
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
 *   idle + empty           → height 0 (nothing)
 *   items + collapsed      → one line (cutout width): #1 task
 *   items + expanded       → wide; top-N rows (drag handle + checkbox + dot + text)
 *   voice: listening       → waveform ribbon (cutout width)
 *   voice: compiling       → pulsing dots (cutout width)
 *   voice: error           → message text (briefly), then idle
 *
 * Single-resize discipline: the drawer's height is reserved INSTANTLY when it
 * changes (no height transition), so `scrollHeight` jumps once → one OS resize.
 * Content fades/slides INSIDE the reserved space via transform/opacity. On
 * collapse the reserved height is held for one retract duration so the exit
 * animation plays, then released in a single step.
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
  expandedWidth: number;
  voiceHotkey: string;
  flashIds: Set<string>;
  reduced: boolean;
  dragId: string | null;
  dragLifted: boolean;
  dragOffsetY: number;
  dropIndex: number | null;
  settleId: string | null;
  rowH: number;
  setListRef: (el: HTMLDivElement) => void;
  onHandleDown: (e: PointerEvent, index: number) => void;
  onMarkDone: (id: string) => void;
  rowTransition: string;
  checkboxTransition: string;
}) {
  // What the drawer should render right now (priority: voice > hover > list).
  // On hover we always "expand" — to the task list when there are items, or to
  // just the add-task hint when the list is empty. Off hover: the #1-task line
  // when there are items, nothing when empty.
  const mode = createMemo<
    "voice" | "expanded" | "collapsed" | "empty"
  >(() => {
    if (props.phase !== null) return "voice";
    if (props.hoverExpanded) return "expanded";
    return props.rows.length === 0 ? "empty" : "collapsed";
  });

  // Target content height (px) for the current mode. 0 collapses the window.
  const targetHeight = createMemo(() => {
    switch (mode()) {
      case "voice":
        // Waveform / dots / error strip — short calm ribbon.
        return Math.round(props.cutoutHeight * 0.65) + 10;
      case "expanded":
        // Task rows (when any) + the always-present add-task hint footer. When
        // the list is empty the hint stands alone.
        return (
          (props.rows.length > 0
            ? props.rows.length * props.rowH + LIST_PAD_Y * 2
            : 0) + HINT_H
        );
      case "collapsed":
        return COLLAPSED_H;
      default:
        return 0;
    }
  });

  // Target content width. The ear row owns its own pinned width; the drawer
  // is independent. Collapsed/voice = cutout width; expanded = wide.
  const targetWidth = createMemo(() => {
    switch (mode()) {
      case "expanded":
        // Wide for the task list; narrow (cutout-width) when it's just the
        // add-task hint, so an empty hover doesn't fan out a near-empty panel.
        return props.rows.length > 0 ? props.expandedWidth : props.cutoutWidth;
      default:
        return props.cutoutWidth;
    }
  });

  // --- Single-resize height reservation ----------------------------------
  // `reservedH` toggles instantly to the target so the window resizes once;
  // on the way DOWN it's held for one retract before releasing to 0.
  const [reservedH, setReservedH] = createSignal(0);
  let collapseTimer: ReturnType<typeof setTimeout> | undefined;
  const clearCollapse = () => {
    if (collapseTimer !== undefined) {
      clearTimeout(collapseTimer);
      collapseTimer = undefined;
    }
  };
  createEffect(() => {
    const target = targetHeight();
    clearCollapse();
    if (target > 0) {
      setReservedH(target);
    } else if (props.reduced) {
      setReservedH(0);
    } else {
      // Hold the height through the exit animation, then drop in one step.
      collapseTimer = setTimeout(() => {
        collapseTimer = undefined;
        setReservedH(0);
      }, RETRACT_MS);
    }
  });
  onCleanup(clearCollapse);

  return (
    <div
      class="overflow-hidden"
      style={{
        // Single-resize discipline (mirrors the old IdleStrip): BOTH width
        // and height jump instantly to their target — NO CSS size transition
        // on the measured element — so `scrollWidth`/`scrollHeight` each step
        // exactly once and the OS window resizes a single time per state
        // change instead of chasing a multi-frame animation. The visible
        // "expand" feel comes from the content fading/rising in (the
        // staggered row reveal), not from animating the window box.
        height: `${reservedH()}px`,
        width: `${targetWidth()}px`,
        background:
          reservedH() > 0 ? "var(--color-notch-surface, #000)" : "transparent",
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
            // Centered under the camera when idle (the user reads the single
            // top task at a glance); the hovered list switches to the
            // left-aligned ExpandedList. `min-w-0` lets a long task shrink +
            // truncate instead of centering off both edges.
            <div
              class="flex items-center justify-center gap-2 px-3"
              classList={{ "notch-flash": props.flashIds.has(item().id) }}
              style={{ height: `${COLLAPSED_H}px` }}
            >
              <span
                class="h-[7px] w-[7px] shrink-0 rounded-full"
                style={{ background: priorityColor(item().priority) }}
                aria-hidden="true"
              />
              <span class="min-w-0 truncate text-[12px] leading-[16px] text-[var(--color-ink)]">
                {item().text}
              </span>
            </div>
          )}
        </Show>
      </Show>

      <Show when={mode() === "expanded" && props.rows.length > 0}>
        <ExpandedList
          rows={props.rows}
          flashIds={props.flashIds}
          reduced={props.reduced}
          dragId={props.dragId}
          dragLifted={props.dragLifted}
          dragOffsetY={props.dragOffsetY}
          dropIndex={props.dropIndex}
          settleId={props.settleId}
          rowH={props.rowH}
          setListRef={props.setListRef}
          onHandleDown={props.onHandleDown}
          onMarkDone={props.onMarkDone}
          rowTransition={props.rowTransition}
          checkboxTransition={props.checkboxTransition}
        />
      </Show>

      <Show when={mode() === "expanded"}>
        <AddHint hotkey={props.voiceHotkey} />
      </Show>
    </div>
  );
}

/* ===========================================================================
 * Add-task hint — the keyboard affordance revealed on hover. Renders the notch
 * voice hotkey (default ⌥⌘L) as keycaps + a short label, so a user hovering an
 * empty notch learns how to capture a task. It's the sole drawer content when
 * the list is empty and a faint footer beneath the rows when it isn't.
 * ======================================================================== */
function AddHint(props: { hotkey: string }) {
  const tokens = createMemo(() => formatHotkeyTokens(props.hotkey));
  const label = createMemo(() => {
    const k = formatHotkey(props.hotkey);
    return k ? `Press ${k} to add a task` : "Add a task by voice";
  });
  return (
    <div
      class="flex items-center justify-center gap-1.5"
      style={{ height: `${HINT_H}px` }}
      role="note"
      aria-label={label()}
    >
      <span class="flex items-center gap-1" aria-hidden="true">
        <For each={tokens()}>
          {(t) => <kbd class="notch-kbd">{t.symbol}</kbd>}
        </For>
      </span>
      <span
        class="text-[11px] leading-none text-[var(--color-ink-faint)]"
        aria-hidden="true"
      >
        to add a task
      </span>
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
 * Expanded list — top-N rows. Each: drag handle + checkbox + priority dot +
 * text. Rows fade+rise in on mount with a ~20ms stagger. Drag lifts a row to
 * follow the pointer; siblings slide; a 2px accent drop-line marks the
 * insertion point.
 * ======================================================================== */
function ExpandedList(props: {
  rows: NotchItem[];
  flashIds: Set<string>;
  reduced: boolean;
  dragId: string | null;
  dragLifted: boolean;
  dragOffsetY: number;
  dropIndex: number | null;
  settleId: string | null;
  rowH: number;
  setListRef: (el: HTMLDivElement) => void;
  onHandleDown: (e: PointerEvent, index: number) => void;
  onMarkDone: (id: string) => void;
  rowTransition: string;
  checkboxTransition: string;
}) {
  const draggingActive = () =>
    props.dragId !== null && props.dragLifted;

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
      class="relative flex flex-col px-2"
      style={{ "padding-top": `${LIST_PAD_Y}px`, "padding-bottom": `${LIST_PAD_Y}px` }}
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
          return (
            <div
              class="flex items-center gap-2 rounded-[8px] px-1.5 hover:bg-white/[0.06]"
              classList={{
                "notch-flash": props.flashIds.has(item.id),
                // Mount-reveal: fade + rise with a per-row stagger. Disabled
                // for any row once a drag is in flight (the lift/shift
                // transform would fight the keyframe's transform) and while a
                // row is flashing (both use the `animation` shorthand).
                "notch-row-enter":
                  !draggingActive() &&
                  !props.reduced &&
                  !props.flashIds.has(item.id),
              }}
              style={{
                height: `${props.rowH}px`,
                "z-index": isDragged() ? 5 : 1,
                position: "relative",
                transform: liftTransform(),
                transition: dragTransition(),
                background: isDragged() ? "rgba(255,255,255,0.10)" : undefined,
                "box-shadow": isDragged()
                  ? "0 6px 16px rgba(0,0,0,0.55)"
                  : "none",
                opacity: 1,
                "animation-delay": draggingActive() ? "0ms" : enterDelay(),
              }}
            >
              {/* Drag handle — always faintly visible when expanded. */}
              <div
                class="notch-drag-handle shrink-0"
                style={{ "touch-action": "none" }}
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

              <button
                type="button"
                aria-label="Mark done"
                onClick={(e) => {
                  e.stopPropagation();
                  props.onMarkDone(item.id);
                }}
                class="flex h-[15px] w-[15px] shrink-0 items-center justify-center rounded-full border"
                style={{
                  "border-color": CHECKBOX_BORDER,
                  transition: props.checkboxTransition,
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
          );
        }}
      </For>
    </div>
  );
}
