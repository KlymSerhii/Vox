import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  getNotchList,
  notchListAdd,
  notchListClearCompleted,
  notchListEdit,
  notchListMarkDone,
  notchListRemove,
  notchListReorder,
  notchListRestore,
  notchListSetPriority,
  onNotchListChanged,
  type NotchItem,
  type NotchListDoc,
  type NotchPriority,
  type Settings,
} from "@/lib/ipc";
import { Card } from "../components/Card";
import { Row } from "../components/Row";
import { SectionHeader } from "../components/SectionHeader";
import { SegmentedControl } from "../components/SegmentedControl";
import { Toggle } from "@/components/Toggle";
import { HotkeyKeys } from "@/components/HotkeyKeys";

const PRIORITY_OPTIONS: { value: NotchPriority; label: string }[] = [
  { value: "high", label: "High" },
  { value: "med", label: "Med" },
  { value: "low", label: "Low" },
];

const DISPLAY_OPTIONS: {
  value: "active" | "builtin" | "primary";
  label: string;
}[] = [
  { value: "active", label: "Active" },
  { value: "builtin", label: "Built-in" },
  { value: "primary", label: "Primary" },
];

// Preset voice-hotkey choices (mirror the backend `NOTCH_VOICE_FALLBACKS`).
const VOICE_HOTKEY_PRESETS = ["Cmd+Alt+L", "Cmd+Shift+L", "Alt+Shift+L"];

function priorityDot(p: NotchPriority): string {
  switch (p) {
    case "high":
      return "var(--color-danger, #e5484d)";
    case "med":
      return "var(--color-warning, #f5a623)";
    default:
      return "var(--color-ink-faint, #8a8a8a)";
  }
}

export function NotchListTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [doc, setDoc] = createSignal<NotchListDoc>({
    version: 1,
    items: [],
    archive: [],
  });
  const [newText, setNewText] = createSignal("");
  const [newPriority, setNewPriority] = createSignal<NotchPriority>("med");
  const [err, setErr] = createSignal<string | null>(null);
  const [dragId, setDragId] = createSignal<string | null>(null);

  let unlisten: (() => void) | null = null;

  async function refresh() {
    try {
      setDoc(await getNotchList());
    } catch (e) {
      console.error("getNotchList:", e);
    }
  }

  onMount(async () => {
    await refresh();
    unlisten = await onNotchListChanged(setDoc);
  });
  onCleanup(() => unlisten?.());

  function activeSorted(): NotchItem[] {
    const rank = (p: NotchPriority) =>
      p === "high" ? 0 : p === "med" ? 1 : 2;
    return doc()
      .items.filter((i) => !i.done)
      .slice()
      .sort((a, b) => rank(a.priority) - rank(b.priority) || a.order - b.order);
  }

  async function addItem() {
    const text = newText().trim();
    if (!text) return;
    setErr(null);
    try {
      await notchListAdd(text, newPriority());
      setNewText("");
    } catch (e) {
      setErr(String(e));
    }
  }

  async function editItem(id: string, text: string) {
    try {
      await notchListEdit(id, text);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function setPriority(id: string, p: NotchPriority) {
    try {
      await notchListSetPriority(id, p);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function complete(id: string) {
    try {
      await notchListMarkDone(id);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function remove(id: string) {
    try {
      await notchListRemove(id);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function restore(id: string) {
    try {
      await notchListRestore(id);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function clearCompleted() {
    try {
      await notchListClearCompleted();
    } catch (e) {
      setErr(String(e));
    }
  }

  // Pointer-based drag-reorder (no library): on drop, send the full
  // active-id order with the dragged item moved to the target slot.
  async function onDropOn(targetId: string) {
    const from = dragId();
    setDragId(null);
    if (!from || from === targetId) return;
    const ids = activeSorted().map((i) => i.id);
    const fromIdx = ids.indexOf(from);
    const toIdx = ids.indexOf(targetId);
    if (fromIdx < 0 || toIdx < 0) return;
    ids.splice(toIdx, 0, ids.splice(fromIdx, 1)[0]);
    try {
      await notchListReorder(ids);
    } catch (e) {
      setErr(String(e));
    }
  }

  return (
    <>
      <SectionHeader title="Notch List" />
      <Card>
        <Row
          label="Show the Notch List"
          hint="A small always-visible to-do list at the top-center of your screen. Hover to expand; click a checkbox to mark a task done."
          last
        >
          <Toggle
            checked={props.s.notch_list_enabled}
            onChange={(v) => props.update({ notch_list_enabled: v })}
          />
        </Row>
      </Card>

      <Show when={props.s.notch_list_enabled}>
        <SectionHeader title="Tasks" />
        <Card>
          {/* New item input */}
          <div class="flex items-center gap-2 px-4 py-3 border-b border-[var(--color-border)]">
            <input
              type="text"
              value={newText()}
              onInput={(e) => setNewText(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void addItem();
              }}
              placeholder="Add a task…"
              class="min-w-0 flex-1 rounded-[7px] bg-[var(--color-control)] px-2.5 py-1.5 text-[13px] text-[var(--color-ink)] ring-1 ring-inset ring-[var(--color-border)] outline-none focus:ring-[var(--color-accent)]"
            />
            <SegmentedControl
              value={newPriority()}
              options={PRIORITY_OPTIONS}
              onChange={setNewPriority}
              ariaLabel="New task priority"
            />
            <button
              type="button"
              onClick={() => void addItem()}
              class="rounded-[7px] bg-[var(--color-accent)] px-3 py-1.5 text-[12px] font-medium text-white"
            >
              Add
            </button>
          </div>

          <Show
            when={activeSorted().length > 0}
            fallback={
              <div class="px-4 py-4 text-[12px] text-[var(--color-ink-mute)]">
                No tasks yet. Add one above, or say it with{" "}
                <HotkeyKeys spec={props.s.notch_list_voice_hotkey} />.
              </div>
            }
          >
            <For each={activeSorted()}>
              {(item, idx) => (
                <div
                  draggable={true}
                  onDragStart={() => setDragId(item.id)}
                  onDragOver={(e) => e.preventDefault()}
                  onDrop={(e) => {
                    e.preventDefault();
                    void onDropOn(item.id);
                  }}
                  class="flex items-center gap-2 px-4 py-2.5"
                  classList={{
                    "border-b border-[var(--color-border)]":
                      idx() < activeSorted().length - 1,
                    "opacity-50": dragId() === item.id,
                  }}
                >
                  <span
                    class="cursor-grab select-none text-[var(--color-ink-faint)]"
                    title="Drag to reorder"
                    aria-hidden="true"
                  >
                    ⠿
                  </span>
                  <button
                    type="button"
                    aria-label="Mark done"
                    onClick={() => void complete(item.id)}
                    class="flex h-[16px] w-[16px] shrink-0 items-center justify-center rounded-full border border-[var(--color-border-strong)] hover:border-[var(--color-accent)]"
                  />
                  <span
                    class="h-[8px] w-[8px] shrink-0 rounded-full"
                    style={{ background: priorityDot(item.priority) }}
                    aria-hidden="true"
                  />
                  <input
                    type="text"
                    value={item.text}
                    onChange={(e) =>
                      void editItem(item.id, e.currentTarget.value)
                    }
                    class="min-w-0 flex-1 bg-transparent text-[13px] text-[var(--color-ink)] outline-none"
                  />
                  <SegmentedControl
                    value={item.priority}
                    options={PRIORITY_OPTIONS}
                    onChange={(p) => void setPriority(item.id, p)}
                    ariaLabel="Task priority"
                  />
                  <button
                    type="button"
                    onClick={() => void remove(item.id)}
                    class="rounded-[6px] px-2 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-danger)]"
                  >
                    Delete
                  </button>
                </div>
              )}
            </For>
          </Show>
        </Card>

        {/* Completed / archive */}
        <Show when={doc().archive.length > 0}>
          <SectionHeader title="Completed" />
          <Card>
            <For each={doc().archive}>
              {(item, idx) => (
                <div
                  class="flex items-center gap-2 px-4 py-2.5"
                  classList={{
                    "border-b border-[var(--color-border)]":
                      idx() < doc().archive.length - 1,
                  }}
                >
                  <span class="truncate text-[13px] text-[var(--color-ink-mute)] line-through flex-1">
                    {item.text}
                  </span>
                  <button
                    type="button"
                    onClick={() => void restore(item.id)}
                    class="rounded-[6px] px-2 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
                  >
                    Restore
                  </button>
                </div>
              )}
            </For>
            <div class="flex justify-end px-4 py-2">
              <button
                type="button"
                onClick={() => void clearCompleted()}
                class="text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-danger)]"
              >
                Clear completed
              </button>
            </div>
          </Card>
        </Show>

        {/* Display + behaviour */}
        <SectionHeader title="Display & behaviour" />
        <Card>
          <Row
            label="Items shown when expanded"
            hint="How many top tasks the notch reveals on hover (3–7)."
          >
            <div class="flex items-center gap-2">
              <button
                type="button"
                onClick={() =>
                  props.update({
                    notch_list_top_n: Math.max(3, props.s.notch_list_top_n - 1),
                  })
                }
                class="h-[24px] w-[24px] rounded-[6px] bg-[var(--color-control)] text-[14px] hover:bg-[var(--color-control-hover)]"
              >
                −
              </button>
              <span class="w-[18px] text-center text-[13px] tabular-nums text-[var(--color-ink)]">
                {props.s.notch_list_top_n}
              </span>
              <button
                type="button"
                onClick={() =>
                  props.update({
                    notch_list_top_n: Math.min(7, props.s.notch_list_top_n + 1),
                  })
                }
                class="h-[24px] w-[24px] rounded-[6px] bg-[var(--color-control)] text-[14px] hover:bg-[var(--color-control-hover)]"
              >
                +
              </button>
            </div>
          </Row>
          <Row
            label="Display"
            hint="Which screen the notch attaches to."
          >
            <SegmentedControl
              value={props.s.notch_list_display}
              options={DISPLAY_OPTIONS}
              onChange={(v) => props.update({ notch_list_display: v })}
              ariaLabel="Notch display"
            />
          </Row>
          <Row
            label="Confirm destructive voice edits"
            hint="When a spoken command removes or clears items, ask for confirmation instead of applying it immediately."
            last
          >
            <Toggle
              checked={props.s.notch_list_confirm_destructive}
              onChange={(v) =>
                props.update({ notch_list_confirm_destructive: v })
              }
            />
          </Row>
        </Card>

        {/* Voice hotkey */}
        <SectionHeader title="Voice editing" />
        <Card>
          <Row
            label="Voice command shortcut"
            hint="Press, speak a command (e.g. “add buy milk”), and Vox edits the list instead of copying text."
            last
          >
            <div class="flex items-center gap-2">
              <HotkeyKeys spec={props.s.notch_list_voice_hotkey} />
              <div class="flex gap-1">
                <For each={VOICE_HOTKEY_PRESETS}>
                  {(spec) => (
                    <button
                      type="button"
                      onClick={() =>
                        props.update({ notch_list_voice_hotkey: spec })
                      }
                      class="rounded-[6px] px-2 py-1 text-[11px] ring-1 ring-inset transition-colors"
                      classList={{
                        "bg-[var(--color-surface-raised)] ring-[var(--color-accent)] text-[var(--color-ink)]":
                          props.s.notch_list_voice_hotkey === spec,
                        "bg-[var(--color-control)] ring-[var(--color-border)] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]":
                          props.s.notch_list_voice_hotkey !== spec,
                      }}
                    >
                      <HotkeyKeys spec={spec} />
                    </button>
                  )}
                </For>
              </div>
            </div>
          </Row>
        </Card>
      </Show>

      <Show when={err()}>
        <p class="text-[11px] text-[var(--color-danger)]">{err()}</p>
      </Show>
    </>
  );
}
