import {
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import {
  getNotchList,
  getPublicSettings,
  notchListMarkDone,
  notchResize,
  onNotchListChanged,
  onSettingsChanged,
  type NotchItem,
  type NotchListDoc,
} from "../lib/ipc";
import { HotkeyKeys } from "../components/HotkeyKeys";

/// Default top-N if settings haven't loaded yet (matches the backend
/// `default_notch_list_top_n`).
const DEFAULT_TOP_N = 5;
/// Default voice hotkey spec (matches `default_notch_list_voice_hotkey`).
const DEFAULT_VOICE_HOTKEY = "Cmd+Alt+L";

/// Collapsed/expanded window heights. Width is content-driven via the
/// ResizeObserver; height is row-driven. Must stay >= the Rust
/// `notch::HEIGHT` floor (30) for the collapsed case.
const COLLAPSED_H = 30;
const ROW_H = 28;
const PANEL_PAD = 12;

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

  let unlistenList: (() => void) | null = null;
  let unlistenSettings: (() => void) | null = null;
  let panelRef: HTMLDivElement | undefined;
  let ro: ResizeObserver | undefined;

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
    } catch (e) {
      console.warn("getPublicSettings (notch):", e);
    }
  }

  /// Push the measured content size up to the OS window so it grows on
  /// hover and snaps back on leave. Mirrors how the pill drives its size.
  /// The `notch_resize` command re-centers the window top-center (the
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
    await refresh();
    unlistenList = await onNotchListChanged(setDoc);
    unlistenSettings = await onSettingsChanged(() => {
      void applySettings();
    });

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
    ro?.disconnect();
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

  return (
    <div
      ref={panelRef}
      class="inline-flex w-max min-w-[160px] max-w-[420px] flex-col overflow-hidden rounded-[12px]"
      style={{
        background: "var(--color-surface-raised, rgba(28,28,32,0.92))",
        "backdrop-filter": "var(--glass-blur)",
        "-webkit-backdrop-filter": "var(--glass-blur)",
        "box-shadow":
          "inset 0 1px 0 var(--glass-highlight), 0 6px 20px rgba(0,0,0,0.35)",
        color: "var(--color-ink)",
      }}
      onMouseEnter={() => {
        setHovered(true);
        queueMicrotask(syncWindowSize);
      }}
      onMouseLeave={() => {
        setHovered(false);
        queueMicrotask(syncWindowSize);
      }}
    >
      {/* Collapsed resting line: #1 task, or the empty-state hint. */}
      <Show
        when={!hovered()}
        fallback={
          // Expanded panel — top-N rows with a checkbox per item.
          <Show
            when={!isEmpty()}
            fallback={<EmptyState voiceHotkey={voiceHotkey()} />}
          >
            <div
              class="flex flex-col gap-0.5 px-2 py-1.5"
              style={{ "min-height": `${PANEL_PAD + ROW_H}px` }}
            >
              <For each={top()}>
                {(item) => (
                  <div
                    class="flex items-center gap-2 rounded-[7px] px-1.5 py-1"
                    style={{ height: `${ROW_H}px`, transition: transition() }}
                  >
                    <button
                      type="button"
                      aria-label="Mark done"
                      onClick={() => void markDone(item.id)}
                      class="flex h-[15px] w-[15px] shrink-0 items-center justify-center rounded-full border transition-colors"
                      style={{
                        "border-color": "var(--color-border, #4a4a4a)",
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
        <Show
          when={headItem()}
          fallback={<EmptyState voiceHotkey={voiceHotkey()} />}
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
      </Show>
    </div>
  );
}

/// Empty-state row. Founder-mandated copy: the literal text
/// "Vox Notch — " followed by the bound voice hotkey rendered through the
/// shared `<HotkeyKeys>` component, followed by " to add a task by voice".
function EmptyState(props: { voiceHotkey: string }) {
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
