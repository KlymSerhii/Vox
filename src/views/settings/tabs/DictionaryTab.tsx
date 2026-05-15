import { createSignal, For, Show } from "solid-js";
import { type Settings } from "@/lib/ipc";
import { Card } from "../components/Card";

export function DictionaryTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [draft, setDraft] = createSignal("");

  function add() {
    const v = draft().trim();
    if (!v) return;
    if (props.s.dictionary.includes(v)) {
      setDraft("");
      return;
    }
    props.update({ dictionary: [...props.s.dictionary, v] });
    setDraft("");
  }

  function remove(term: string) {
    props.update({
      dictionary: props.s.dictionary.filter((t) => t !== term),
    });
  }

  return (
    <>
      <Card>
        <div class="px-4 py-3">
          <p class="mb-3 text-[11px] text-[var(--color-ink-mute)]">
            Names, acronyms, and technical terms Vox should spell correctly.
            Applied during STT bias and the cleanup step.
          </p>
          <div class="mb-3 flex gap-2">
            <input
              type="text"
              class="flex-1 rounded-[6px] bg-[var(--color-input-bg)] px-3 py-2 text-[13px] ring-1 ring-[var(--color-border)] outline-none focus:ring-2 focus:ring-[var(--color-accent)]"
              placeholder="Anthropic, kubectl, OAuth…"
              value={draft()}
              onInput={(e) => setDraft(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  add();
                }
              }}
            />
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)] px-4 py-2 text-[12px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:opacity-50"
              disabled={!draft().trim()}
              onClick={add}
            >
              Add
            </button>
          </div>
          <Show
            when={props.s.dictionary.length > 0}
            fallback={
              <p class="text-[12px] text-[var(--color-ink-mute)]">
                Add a word once and Vox will spell <em>Anthropic</em> even if
                you sound like you're saying <em>Anthropik</em>.
              </p>
            }
          >
            <div class="flex flex-wrap gap-2">
              <For each={props.s.dictionary}>
                {(term) => (
                  <button
                    type="button"
                    onClick={() => remove(term)}
                    class="group inline-flex items-center gap-1 rounded-[6px] bg-[var(--color-control)] px-2 py-1 text-[12px] hover:bg-[var(--color-control-hover)]"
                    title="Click to remove"
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
      <p class="text-[11px] text-[var(--color-ink-mute)]">
        {props.s.dictionary.length} term
        {props.s.dictionary.length === 1 ? "" : "s"}.
      </p>
    </>
  );
}
