import {
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

/// Slim height of the idle "nub" — the resting state when the list is empty.
/// Deliberately tiny so it reads as a thin tab hanging under the camera bump
/// (the founder ask: "very little space vertically").
const NUB_H = 14;
/// Fallback nub width (logical px) when the real camera-bump width is
/// unknown — a notchless display, or before the geometry read resolves.
const DEFAULT_NUB_WIDTH = 168;
/// Height of the collapsed "show #1 task" bar and the empty-state hover hint.
/// These need room for a line of text, unlike the idle nub. The window is
/// content-driven via the ResizeObserver, so this is just the row height.
const COLLAPSED_H = 30;
const ROW_H = 28;
const PANEL_PAD = 12;

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

  /// Idle-nub width: the real camera-bump width when known, else a sensible
  /// default. Rounded so the window doesn't request sub-pixel sizes, and
  /// sanity-bounded (a physical notch is ~180–230 pt) so a bad geometry read
  /// can never stretch the nub to an absurd width.
  const nubWidth = createMemo(() => {
    const w = geo().width;
    return w >= 80 && w <= 360 ? Math.round(w) : DEFAULT_NUB_WIDTH;
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
    <div
      ref={panelRef}
      class="inline-flex w-max max-w-[440px] flex-col overflow-hidden rounded-[14px]"
      style={{
        // Always near-black so the panel reads as an extension of the
        // physical (true-black) notch, regardless of app appearance. All
        // tokens here come from the notch's own stylesheet (`src/notch.css`),
        // never the pill/app design system.
        background: "var(--color-notch-surface, rgba(10,10,12,0.92))",
        "backdrop-filter": "var(--notch-blur, blur(20px) saturate(150%))",
        "-webkit-backdrop-filter": "var(--notch-blur, blur(20px) saturate(150%))",
        // Faint top rim for definition over dark wallpapers; the OS window
        // shadow (hasShadow=YES) supplies the drop shadow under the panel.
        "box-shadow":
          "inset 0 1px 0 rgba(255,255,255,0.06), 0 8px 24px rgba(0,0,0,0.55)",
        color: "var(--color-ink)",
      }}
      tabindex={clickToExpand() ? 0 : undefined}
      onMouseEnter={() => {
        if (clickToExpand()) return;
        armHoverExpand();
      }}
      onMouseLeave={() => {
        if (clickToExpand()) return;
        collapse();
      }}
      onFocusOut={(e) => {
        // Click-to-expand: collapse when focus leaves the panel (a click
        // elsewhere on the desktop). Hover mode ignores it. We use
        // `focusout` (which bubbles, unlike `blur`) so this still fires
        // when focus was on a descendant — and we ignore moves that stay
        // INSIDE the panel (e.g. clicking a row's "mark done" checkbox),
        // which would otherwise collapse the panel the instant the user
        // ticks the first item.
        if (!clickToExpand()) return;
        const next = e.relatedTarget as Node | null;
        if (next && panelRef?.contains(next)) return;
        collapse();
      }}
    >
      {/* Collapsed resting state: the idle nub, or the #1 task line. */}
      <Show
        when={!hovered()}
        fallback={
          // Expanded panel — top-N rows with a checkbox per item, or the
          // empty-state hint when there's nothing to show.
          <Show
            when={!isEmpty()}
            fallback={<EmptyHint voiceHotkey={voiceHotkey()} />}
          >
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
          </Show>
        }
      >
        {/* Click-to-expand wrapper: one handler covers both the idle nub and
            the #1-task bar so they expand identically on click. */}
        <div
          classList={{ "cursor-pointer": clickToExpand() }}
          onClick={toggleClickExpand}
        >
          <Show
            when={headItem()}
            fallback={<Nub width={nubWidth()} />}
          >
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
  );
}

/// Idle nub — the resting state when the list is empty. A slim black tab the
/// width of the camera bump with a faint centered grabber that hints "there's
/// something here". Hover (or click in click-to-expand mode) reveals the
/// full hotkey hint via `EmptyHint`.
function Nub(props: { width: number }) {
  return (
    <div
      class="flex items-center justify-center"
      style={{ width: `${props.width}px`, height: `${NUB_H}px` }}
      aria-label="Vox Notch"
    >
      <span
        style={{
          width: "26px",
          height: "3px",
          "border-radius": "999px",
          background: "rgba(255,255,255,0.22)",
        }}
        aria-hidden="true"
      />
    </div>
  );
}

/// Empty-state hover hint. Founder-mandated copy: the literal text
/// "Vox Notch — " followed by the bound voice hotkey rendered through the
/// shared `<HotkeyKeys>` component, followed by " to add a task by voice".
/// Shown when the list is empty AND the panel is expanded (hover/click) —
/// the idle resting state is the slim `Nub`.
function EmptyHint(props: { voiceHotkey: string }) {
  return (
    <div
      class="flex items-center gap-1.5 px-3 text-[11px] text-[var(--color-ink-mute)]"
      style={{ height: `${COLLAPSED_H}px` }}
    >
      <span>Vox Notch&nbsp;—&nbsp;</span>
      <HotkeyKeys spec={props.voiceHotkey} />
      <span>&nbsp;to add a task by voice</span>
    </div>
  );
}
