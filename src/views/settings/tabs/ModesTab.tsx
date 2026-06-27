import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  deleteMode,
  duplicateMode,
  exportMode,
  importModeJson,
  listModes,
  onModesChanged,
  resetDefaultModes,
  saveMode,
  type Settings,
  type VoiceMode,
} from "@/lib/ipc";
import { Card } from "../components/Card";
import { Row } from "../components/Row";
import { useConfirm } from "../useConfirm";
import { ModeEditor } from "./ModeEditor";
import { ModeRow } from "./ModeRow";

export function ModesTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [modes, setModes] = createSignal<VoiceMode[]>([]);
  const [editing, setEditing] = createSignal<VoiceMode | null>(null);
  const [importErr, setImportErr] = createSignal<string | null>(null);
  const [resetMsg, setResetMsg] = createSignal<string | null>(null);
  const [saveErr, setSaveErr] = createSignal<string | null>(null);
  // `window.confirm` is a silent no-op in Tauri's WKWebView, so the
  // reset-to-defaults button uses an inline confirm with 5 s auto-revert.
  // Per-row delete confirms live inside ModeRow.
  const confirmReset = useConfirm();
  let unlistenModes: (() => void) | null = null;
  onCleanup(() => {
    unlistenModes?.();
  });
  let fileInput: HTMLInputElement | undefined;

  async function refresh() {
    try {
      setModes(await listModes());
    } catch (e) {
      console.error("listModes:", e);
    }
  }

  onMount(async () => {
    await refresh();
    // The backend re-broadcasts the list on every mutation. Subscribing
    // here means a mode created from the Tray popover (future) or a
    // second Settings window stays in sync without polling.
    unlistenModes = await onModesChanged(setModes);
  });

  function onDragOver(e: DragEvent) {
    e.preventDefault();
  }
  async function onDrop(e: DragEvent) {
    e.preventDefault();
    const file = e.dataTransfer?.files?.[0];
    if (!file) return;
    if (!file.name.endsWith(".voxmode") && !file.name.endsWith(".json")) {
      setImportErr("Only .voxmode or .json files are supported.");
      return;
    }
    const text = await file.text();
    await doImport(text);
  }

  async function doImport(text: string) {
    setImportErr(null);
    try {
      await importModeJson(text);
      // `modes:changed` fires from the backend on success; the
      // listener below updates the local list.
    } catch (e) {
      setImportErr(String(e));
    }
  }

  async function onFilePick(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    const text = await file.text();
    input.value = "";
    await doImport(text);
  }

  async function onActivate(id: string) {
    props.update({ active_mode_id: id });
  }

  async function onDuplicate(id: string) {
    try {
      const m = await duplicateMode(id);
      // The local list refreshes via the `modes:changed` listener;
      // `m` is the canonical backend object so the editor can open
      // even before the event has been processed.
      setEditing(m);
    } catch (e) {
      console.error("duplicate:", e);
    }
  }

  async function onDelete(id: string) {
    try {
      await deleteMode(id);
      if (editing()?.id === id) setEditing(null);
    } catch (e) {
      console.error("delete:", e);
    }
  }

  async function onExport(m: VoiceMode) {
    try {
      const json = await exportMode(m.id);
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${m.id}.voxmode`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 0);
    } catch (e) {
      console.error("export:", e);
    }
  }

  async function onReset() {
    setResetMsg(null);
    try {
      const n = await resetDefaultModes();
      if (n === 0) {
        setResetMsg("All defaults are already present.");
        setTimeout(() => setResetMsg(null), 4000);
      } else {
        setResetMsg(`Restored ${n} default mode${n === 1 ? "" : "s"}.`);
        setTimeout(() => setResetMsg(null), 4000);
      }
    } catch (e) {
      console.error("reset:", e);
    }
  }

  function onNew() {
    const draft: VoiceMode = {
      id: "",
      name: "New Mode",
      icon: "🌟",
      description: "",
      system_prompt:
        "You clean up dictated text. Return plain text only — no preface.",
      dictionary: [],
      punctuation: "Standard",
      case: "Sentence",
      max_length_chars: null,
      context_enabled: true,
      auto_triggers: [],
      built_in: false,
      created_at: 0,
      updated_at: 0,
    };
    setEditing(draft);
  }

  async function onSaveDraft(mode: VoiceMode) {
    let id = mode.id;
    if (!id) {
      // Backend dedups if this collides with an existing mode.
      id = mode.name
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/(^-|-$)/g, "");
      if (!id) id = `mode-${Date.now()}`;
    }
    // Strip trigger rows where every field is blank — those are
    // half-filled drafts, not real triggers.
    const cleanedTriggers = mode.auto_triggers.filter((t) => {
      const bundle = (t.bundle_id_glob ?? "").trim();
      const title = (t.window_title_substring ?? "").trim();
      return bundle.length > 0 || title.length > 0;
    });
    const toSave: VoiceMode = { ...mode, id, auto_triggers: cleanedTriggers };
    try {
      await saveMode(toSave);
      setEditing(null);
    } catch (e) {
      console.error("save mode:", e);
      // Surface the error inline rather than via window.alert (no-op in
      // WKWebView). The editor stays open so the user can adjust + retry.
      setSaveErr(String(e));
    }
  }

  return (
    <div onDragOver={onDragOver} onDrop={onDrop}>
      <Card>
        <Row
          label="Active mode"
          hint="Used for the next utterance unless you cycle or pick a different mode."
          last
        >
          <select
            class="rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
            value={props.s.active_mode_id}
            onChange={(e) => onActivate(e.currentTarget.value)}
          >
            <For each={modes()}>
              {(m) => (
                <option value={m.id}>
                  {m.icon} {m.name}
                </option>
              )}
            </For>
          </select>
        </Row>
      </Card>

      <Show when={importErr()}>
        <div class="mb-3 rounded-[10px] bg-[var(--color-danger)]/10 p-3 text-[12px] text-[var(--color-danger)] ring-1 ring-[var(--color-danger)]/40">
          Import failed: {importErr()}
        </div>
      </Show>

      <For each={modes()}>
        {(m) => (
          <ModeRow
            m={m}
            isActive={m.id === props.s.active_mode_id}
            onActivate={() => onActivate(m.id)}
            onEdit={() => setEditing(m)}
            onDuplicate={() => onDuplicate(m.id)}
            onDelete={() => onDelete(m.id)}
            onExport={() => onExport(m)}
          />
        )}
      </For>

      <div class="mt-2 flex flex-wrap gap-2">
        <button
          type="button"
          onClick={onNew}
          class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1.5 text-[12px] font-medium text-[var(--color-on-accent)] hover:brightness-110"
        >
          + New mode
        </button>
        <button
          type="button"
          onClick={() => fileInput?.click()}
          class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
        >
          Import .voxmode
        </button>
        <Show
          when={!confirmReset.armed()}
          fallback={
            <>
              <span class="self-center text-[11px] text-[var(--color-ink-mute)]">
                Restore built-in modes?
              </span>
              <button
                type="button"
                onClick={confirmReset.disarm}
                class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => confirmReset.armOrFire(() => void onReset())}
                class="rounded-[6px] bg-[var(--color-danger)] px-3 py-1.5 text-[12px] font-medium text-white hover:brightness-110"
              >
                Reset
              </button>
            </>
          }
        >
          <button
            type="button"
            onClick={() => confirmReset.armOrFire(() => void onReset())}
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            Reset to defaults
          </button>
        </Show>
        <input
          ref={fileInput}
          type="file"
          accept=".voxmode,.json,application/json"
          class="hidden"
          onChange={onFilePick}
        />
      </div>
      <Show when={resetMsg()}>
        <p class="mt-2 text-[11px] text-[var(--color-ink-mute)]">{resetMsg()}</p>
      </Show>
      <p class="mt-3 text-[11px] text-[var(--color-ink-mute)]">
        Drag a <code class="text-[var(--color-ink)]">.voxmode</code> file
        anywhere on this tab to import. Built-in modes can't be edited
        directly — duplicate one to make changes.
      </p>

      <Show when={editing()}>
        {(m) => (
          <ModeEditor
            mode={m()}
            llmEngine={props.s.llm_engine}
            saveError={saveErr()}
            onCancel={() => {
              setSaveErr(null);
              setEditing(null);
            }}
            onSave={(mode) => {
              setSaveErr(null);
              return onSaveDraft(mode);
            }}
          />
        )}
      </Show>
    </div>
  );
}
