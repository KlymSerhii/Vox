import { createSignal, For, Show } from "solid-js";
import {
  type CaseStyle,
  type PunctuationStyle,
  type VoiceMode,
} from "@/lib/ipc";
import { Card } from "../components/Card";
import { Row } from "../components/Row";
import { Toggle } from "@/components/Toggle";
import { CASE_OPTIONS, PUNCTUATION_OPTIONS } from "../constants";

export function ModeEditor(props: {
  mode: VoiceMode;
  llmEngine: string;
  saveError: string | null;
  onCancel: () => void;
  onSave: (m: VoiceMode) => void;
}) {
  const [draft, setDraft] = createSignal<VoiceMode>({ ...props.mode });
  const [dictDraft, setDictDraft] = createSignal("");
  // Toggled true on a save attempt with an empty required field so the
  // inline error renders without flashing on every keystroke.
  const [showValidation, setShowValidation] = createSignal(false);
  const nameInvalid = () => draft().name.trim().length === 0;
  const promptInvalid = () => draft().system_prompt.trim().length === 0;
  const canSave = () => !nameInvalid() && !promptInvalid();

  function patch(partial: Partial<VoiceMode>) {
    setDraft({ ...draft(), ...partial });
  }

  function addDictTerm() {
    const v = dictDraft().trim();
    if (!v) return;
    if (draft().dictionary.includes(v)) {
      setDictDraft("");
      return;
    }
    patch({ dictionary: [...draft().dictionary, v] });
    setDictDraft("");
  }

  function removeDictTerm(t: string) {
    patch({ dictionary: draft().dictionary.filter((d) => d !== t) });
  }

  function addTrigger() {
    patch({
      auto_triggers: [
        ...draft().auto_triggers,
        { bundle_id_glob: "", window_title_substring: null },
      ],
    });
  }

  function updateTrigger(idx: number, field: "bundle_id_glob" | "window_title_substring", value: string) {
    const triggers = draft().auto_triggers.map((t, i) => {
      if (i !== idx) return t;
      const next = { ...t };
      next[field] = value || null;
      return next;
    });
    patch({ auto_triggers: triggers });
  }

  function removeTrigger(idx: number) {
    patch({
      auto_triggers: draft().auto_triggers.filter((_, i) => i !== idx),
    });
  }

  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-[var(--color-scrim)] px-6 py-8">
      <div
        class="flex h-full w-full max-w-2xl flex-col overflow-hidden rounded-[14px] ring-1 ring-[var(--color-border-strong)]"
        style={{
          background: "var(--color-surface-raised)",
          "backdrop-filter": "var(--glass-blur)",
          "-webkit-backdrop-filter": "var(--glass-blur)",
          "box-shadow": "var(--glass-shadow-stack)",
        }}
      >
        <header class="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
          <h2 class="text-[14px] font-semibold">
            {props.mode.id ? "Edit mode" : "New mode"}
          </h2>
          <button
            type="button"
            class="rounded p-1 text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]"
            onClick={props.onCancel}
            aria-label="Cancel"
          >
            ×
          </button>
        </header>
        <div class="flex-1 overflow-y-auto scrollbar-thin px-4 py-4">
          <Card>
            <Row label="Name">
              <div class="flex flex-col items-end gap-1">
                <input
                  type="text"
                  class="w-48 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                  classList={{
                    "ring-[var(--color-danger)]":
                      showValidation() && nameInvalid(),
                  }}
                  value={draft().name}
                  onInput={(e) => patch({ name: e.currentTarget.value })}
                />
                <Show when={showValidation() && nameInvalid()}>
                  <span class="text-[11px] text-[var(--color-danger)]">
                    Name can't be empty.
                  </span>
                </Show>
              </div>
            </Row>
            <Row label="Icon (emoji or text)">
              <input
                type="text"
                class="w-24 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-center text-[14px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().icon}
                onInput={(e) => patch({ icon: e.currentTarget.value })}
              />
            </Row>
            <Row label="Description" last>
              <input
                type="text"
                class="w-64 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().description}
                onInput={(e) => patch({ description: e.currentTarget.value })}
              />
            </Row>
          </Card>

          <Card>
            <div class="px-4 py-3">
              <div class="mb-1 text-[12px] font-medium">System prompt</div>
              <textarea
                class="h-48 w-full rounded-[6px] bg-[var(--color-input-bg)] px-2 py-2 font-mono text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                classList={{
                  "ring-[var(--color-danger)]":
                    showValidation() && promptInvalid(),
                }}
                value={draft().system_prompt}
                onInput={(e) => patch({ system_prompt: e.currentTarget.value })}
              />
              <p class="mt-1 text-[11px] text-[var(--color-ink-mute)]">
                {draft().system_prompt.length} chars. Tip: keep it tight
                and end with explicit "Output only the cleaned text."
              </p>
              <Show when={showValidation() && promptInvalid()}>
                <p class="mt-1 text-[11px] text-[var(--color-danger)]">
                  System prompt can't be empty.
                </p>
              </Show>
            </div>
          </Card>

          <Card>
            <Row
              label="Thinking"
              hint={
                "Override the global Thinking setting for this mode. Only applies when polishing with Gemma." +
                (props.llmEngine !== "gemma-local"
                  ? " (No effect — current polishing engine isn't Gemma.)"
                  : "")
              }
              last
            >
              <select
                class="rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)] disabled:cursor-not-allowed disabled:opacity-50"
                disabled={props.llmEngine !== "gemma-local"}
                value={
                  draft().enable_thinking === true
                    ? "on"
                    : draft().enable_thinking === false
                      ? "off"
                      : "inherit"
                }
                onChange={(e) => {
                  const v = e.currentTarget.value;
                  patch({
                    enable_thinking:
                      v === "on" ? true : v === "off" ? false : null,
                  });
                }}
              >
                <option value="inherit">Inherit from settings</option>
                <option value="on">Always on</option>
                <option value="off">Always off</option>
              </select>
            </Row>
          </Card>

          <Card>
            <Row
              label="Use my context"
              hint="Include your personal context (from the Dictionary & Context tab) in this mode's cleanup."
              last
            >
              <Toggle
                checked={draft().context_enabled ?? true}
                onChange={(v) => patch({ context_enabled: v })}
              />
            </Row>
          </Card>

          <Card>
            <Row label="Punctuation">
              <select
                class="rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().punctuation}
                onChange={(e) =>
                  patch({
                    punctuation: e.currentTarget.value as PunctuationStyle,
                  })
                }
              >
                <For each={PUNCTUATION_OPTIONS}>
                  {(o) => <option value={o.value}>{o.label}</option>}
                </For>
              </select>
            </Row>
            <Row label="Case">
              <select
                class="rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().case}
                onChange={(e) =>
                  patch({ case: e.currentTarget.value as CaseStyle })
                }
              >
                <For each={CASE_OPTIONS}>
                  {(o) => <option value={o.value}>{o.label}</option>}
                </For>
              </select>
            </Row>
            <Row
              label="Max length"
              hint="0 or empty = no cap. Truncates at the last sentence boundary that fits."
              last
            >
              <input
                type="number"
                min="0"
                class="w-20 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                value={draft().max_length_chars ?? ""}
                onInput={(e) => {
                  const v = e.currentTarget.value.trim();
                  patch({ max_length_chars: v === "" ? null : Number(v) });
                }}
              />
            </Row>
          </Card>

          <Card>
            <div class="px-4 py-3">
              <div class="mb-2 text-[12px] font-medium">
                Mode dictionary
              </div>
              <p class="mb-2 text-[11px] text-[var(--color-ink-mute)]">
                Appended to the global dictionary at runtime.
              </p>
              <div class="mb-2 flex gap-2">
                <input
                  type="text"
                  class="flex-1 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[12px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                  placeholder="Add proper nouns, acronyms, or technical terms — one per line."
                  value={dictDraft()}
                  onInput={(e) => setDictDraft(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      addDictTerm();
                    }
                  }}
                />
                <button
                  type="button"
                  class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[12px] hover:bg-[var(--color-control-hover)]"
                  onClick={addDictTerm}
                >
                  Add
                </button>
              </div>
              <Show when={draft().dictionary.length > 0}>
                <div class="flex flex-wrap gap-2">
                  <For each={draft().dictionary}>
                    {(term) => (
                      <button
                        type="button"
                        onClick={() => removeDictTerm(term)}
                        class="group inline-flex items-center gap-1 rounded-[6px] bg-[var(--color-control)] px-2 py-1 text-[12px] hover:bg-[var(--color-control-hover)]"
                      >
                        <span>{term}</span>
                        <span class="text-[var(--color-ink-mute)] group-hover:text-[var(--color-danger)]">
                          ×
                        </span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </Card>

          <Card>
            <div class="px-4 py-3">
              <div class="mb-2 flex items-center justify-between">
                <span class="text-[12px] font-medium">
                  Auto-triggers (suggest only)
                </span>
                <button
                  type="button"
                  class="rounded-[6px] bg-[var(--color-control)] px-2 py-1 text-[11px] hover:bg-[var(--color-control-hover)]"
                  onClick={addTrigger}
                >
                  + Trigger
                </button>
              </div>
              <p class="mb-2 text-[11px] text-[var(--color-ink-mute)]">
                Vox never auto-switches. These rules feed the suggestion
                HUD on the listening pill. Trailing wildcards allowed
                (e.g. <code>com.jetbrains.*</code>).
              </p>
              <For each={draft().auto_triggers}>
                {(t, i) => (
                  <div class="mb-2 flex items-center gap-2">
                    <input
                      type="text"
                      class="flex-1 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[11px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                      placeholder="bundle id (e.g. com.apple.Notes)"
                      value={t.bundle_id_glob ?? ""}
                      onInput={(e) =>
                        updateTrigger(
                          i(),
                          "bundle_id_glob",
                          e.currentTarget.value,
                        )
                      }
                    />
                    <input
                      type="text"
                      class="flex-1 rounded-[6px] bg-[var(--color-input-bg)] px-2 py-1 text-[11px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
                      placeholder="window title contains (optional)"
                      value={t.window_title_substring ?? ""}
                      onInput={(e) =>
                        updateTrigger(
                          i(),
                          "window_title_substring",
                          e.currentTarget.value,
                        )
                      }
                    />
                    <button
                      type="button"
                      class="rounded-[6px] bg-[var(--color-control)] px-2 py-1 text-[11px] hover:bg-[var(--color-danger)]/20 hover:text-[var(--color-danger)]"
                      onClick={() => removeTrigger(i())}
                    >
                      ×
                    </button>
                  </div>
                )}
              </For>
            </div>
          </Card>
        </div>
        <footer class="flex items-center justify-end gap-2 border-t border-[var(--color-border)] px-4 py-3">
          <Show when={props.saveError}>
            <span class="mr-auto text-[11px] text-[var(--color-danger)]">
              {props.saveError}
            </span>
          </Show>
          <button
            type="button"
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
            onClick={props.onCancel}
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!canSave()}
            class="rounded-[6px] bg-[var(--color-accent)] px-4 py-1.5 text-[12px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
            onClick={() => {
              if (!canSave()) {
                setShowValidation(true);
                return;
              }
              props.onSave(draft());
            }}
          >
            Save
          </button>
        </footer>
      </div>
    </div>
  );
}
