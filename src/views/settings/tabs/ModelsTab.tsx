import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  appleFmAvailability,
  cancelDownload,
  deleteModel,
  downloadModel,
  listModels,
  onModelCanceled,
  onModelDone,
  onModelError,
  onModelProgress,
  openExternalUrl,
  parakeetAvailability,
  parakeetSupportsLanguage,
  type AppleFmAvailability,
  type ModelStatus,
  type ParakeetAvailability,
  type Settings,
} from "@/lib/ipc";
import { Card } from "../components/Card";
import { Row } from "../components/Row";
import { SectionHeader } from "../components/SectionHeader";
import { StaticValue } from "../components/StaticValue";
import { Toggle } from "@/components/Toggle";
import { AppleFmEngineRow } from "./AppleFmEngineRow";
import { GemmaEngineRow } from "./GemmaEngineRow";
import { InferenceServerSection } from "./InferenceServerSection";
import { ModelRow } from "./ModelRow";
import { SttEngineCard } from "./SttEngineCard";

export function ModelsTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [models, setModels] = createSignal<ModelStatus[]>([]);
  const [progress, setProgress] = createSignal<
    Record<string, { downloaded: number; total: number; eta: number | null }>
  >({});
  const [downloading, setDownloading] = createSignal<Set<string>>(new Set());

  async function refresh() {
    try {
      setModels(await listModels());
    } catch (e) {
      console.error("listModels:", e);
    }
  }

  let unlistens: (() => void)[] = [];
  onMount(async () => {
    await refresh();
    unlistens.push(
      await onModelProgress((p) => {
        setProgress((prev) => ({
          ...prev,
          [p.id]: {
            downloaded: p.bytes_downloaded,
            total: p.total_bytes,
            eta: p.eta_secs,
          },
        }));
      }),
    );
    const dropDownloading = (id: string) =>
      setDownloading((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    const dropProgress = (id: string) =>
      setProgress((prev) => {
        const { [id]: _, ...rest } = prev;
        return rest;
      });

    unlistens.push(
      await onModelDone(async (p) => {
        dropDownloading(p.id);
        dropProgress(p.id);
        await refresh();
      }),
    );
    unlistens.push(
      await onModelError((p) => {
        console.error("model error", p);
        dropDownloading(p.id);
      }),
    );
    unlistens.push(
      await onModelCanceled((p) => {
        dropDownloading(p.id);
        dropProgress(p.id);
      }),
    );
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
  });

  async function startDownload(id: string) {
    setDownloading((prev) => new Set(prev).add(id));
    try {
      await downloadModel(id);
    } catch (e) {
      console.error("download:", e);
      setDownloading((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }

  async function cancel(id: string) {
    await cancelDownload(id);
  }

  // ModelRow gates the destructive call behind an inline two-step
  // confirm — the native browser `window.confirm()` was silently
  // returning false inside Tauri's WKWebView, so the previous
  // `if (!confirm(...)) return` short-circuited every time and the
  // delete never fired.
  async function remove(id: string) {
    try {
      await deleteModel(id);
    } catch (e) {
      console.error("delete:", e);
    }
    await refresh();
  }

  const [appleFm, setAppleFm] = createSignal<AppleFmAvailability | null>(null);
  const [parakeet, setParakeet] = createSignal<ParakeetAvailability | null>(
    null,
  );

  async function refreshAppleFm() {
    try {
      setAppleFm(await appleFmAvailability());
    } catch (e) {
      console.warn("appleFmAvailability:", e);
    }
  }

  async function refreshParakeet() {
    try {
      setParakeet(await parakeetAvailability());
    } catch (e) {
      console.warn("parakeetAvailability:", e);
    }
  }

  onMount(() => {
    refreshAppleFm();
    refreshParakeet();
  });

  // Switching to a model from a different STT engine flips both fields
  // in one update so session.rs always sees a consistent (engine, model)
  // pair — otherwise a transient mismatch would route the next session
  // to the wrong engine.
  function setActiveStt(engine: string, modelId: string) {
    props.update({ stt_engine: engine, stt_model_id: modelId });
  }

  // One-shot upgrade path: download Parakeet (if not already on disk)
  // and flip the engine. The Use button on the Parakeet model row
  // handles the post-download switch; this just kicks the download.
  async function tryParakeet() {
    const target = sttModels().find(
      (m) => m.descriptor.id === "parakeet-tdt-0.6b-v3",
    );
    if (!target) return;
    if (target.installed) {
      setActiveStt("parakeet-local", target.descriptor.id);
      return;
    }
    await startDownload(target.descriptor.id);
  }

  function setActiveLlm(modelId: string) {
    // Switching to a Gemma model implicitly switches the engine too —
    // a user clicking "Use" on a Gemma row probably wants it active,
    // not "selected for some future toggle".
    props.update({ llm_engine: "gemma-local", llm_model_id: modelId });
  }

  function setEngineAppleFm() {
    // Re-probe right before switching so we surface the current
    // availability error if the user toggled Apple Intelligence off in
    // System Settings since the tab was last opened.
    refreshAppleFm();
    props.update({ llm_engine: "apple-fm" });
  }

  function setEngineGemma() {
    // Use the existing llm_model_id if it still maps to a Gemma model,
    // otherwise the field stays as-is and the user picks one below.
    props.update({ llm_engine: "gemma-local" });
  }

  const sttModels = createMemo(() =>
    models().filter((m) => m.descriptor.kind === "stt"),
  );
  const llmModels = createMemo(() =>
    models().filter((m) => m.descriptor.kind === "llm"),
  );

  // `polishing_enabled` carries user *intent* across LLM outages so we
  // can restore the engine when it stabilises. `llm_engine` carries the
  // engine choice (apple-fm vs gemma-local) and survives the toggle —
  // session.rs short-circuits to the no-op when polishing_enabled is
  // false, so we don't need to clobber the field on toggle off.
  function setPolishing(on: boolean) {
    const next: Partial<Settings> = { polishing_enabled: on };
    if (on && props.s.llm_engine === "none") {
      // Cloud-migration / fresh-install rescue: pick a sensible engine
      // when turning polishing back on with no engine remembered.
      next.llm_engine = appleFm()?.available ? "apple-fm" : "gemma-local";
    }
    props.update(next);
  }

  const activeStt = createMemo(() =>
    sttModels().find((m) => m.descriptor.id === props.s.stt_model_id),
  );
  const activeLlm = createMemo(() =>
    llmModels().find((m) => m.descriptor.id === props.s.llm_model_id),
  );

  // Partition the STT catalog by engine so the two engine cards can
  // each render only their own model rows.
  const parakeetModels = createMemo(() =>
    sttModels().filter((m) => m.descriptor.engine === "parakeet-local"),
  );
  const whisperModels = createMemo(() =>
    sttModels().filter((m) => m.descriptor.engine === "whisper-local"),
  );

  // The "Try Parakeet" upgrade prompt only fires when (a) the user
  // hasn't dismissed it, (b) Parakeet is actually available on this
  // Mac, (c) the user is currently on Whisper, and (d) their active
  // language sits inside Parakeet's 25-language coverage. "auto"
  // qualifies — Parakeet handles auto-detect across the supported set.
  const showTryParakeet = createMemo(() => {
    if (props.s.try_parakeet_dismissed) return false;
    if (props.s.stt_engine !== "whisper-local") return false;
    if (!parakeet()?.available) return false;
    const lang = props.s.language;
    if (lang === "auto") return true;
    return parakeetSupportsLanguage(lang);
  });

  function dismissTryParakeet() {
    props.update({ try_parakeet_dismissed: true });
  }

  return (
    <>
      <SectionHeader
        title="Speech recognition"
        hint={
          props.s.stt_engine === "parakeet-local"
            ? "Local · Parakeet"
            : "Local · Whisper"
        }
      />
      <Card>
        <Row
          label="Active model"
          hint={
            activeStt()
              ? `${activeStt()!.descriptor.name} · ${activeStt()!.installed ? "installed" : "not yet downloaded"}`
              : "No model selected. Pick one below."
          }
          last
        >
          <StaticValue>
            {activeStt() ? activeStt()!.descriptor.tier : "—"} tier
          </StaticValue>
        </Row>
      </Card>

      <Show when={showTryParakeet()}>
        <div class="mb-3 flex items-start justify-between gap-3 rounded-[10px] bg-[var(--color-accent-soft)] p-3 ring-1 ring-[var(--color-accent)]/40">
          <div class="flex-1 text-[12px] text-[var(--color-ink)]">
            <span class="font-medium">Try Parakeet</span> — typically
            5-10× faster on your language. Runs on the Apple Neural
            Engine.
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:brightness-110"
              onClick={tryParakeet}
            >
              Switch to Parakeet
            </button>
            <button
              type="button"
              aria-label="Dismiss"
              title="Dismiss"
              class="grid h-6 w-6 place-items-center rounded-[6px] text-[14px] leading-none text-[var(--color-ink-mute)] hover:bg-[var(--color-control-hover)] hover:text-[var(--color-ink)]"
              onClick={dismissTryParakeet}
            >
              ✕
            </button>
          </div>
        </div>
      </Show>

      <Show when={parakeet()}>
        {(pa) => (
          <SttEngineCard
            title="Parakeet (NVIDIA)"
            subtitle="NVIDIA Parakeet TDT 0.6B v3 — multilingual, 25 languages, ANE-accelerated."
            recommendation="Recommended for English and European languages."
            engineId="parakeet-local"
            isActive={props.s.stt_engine === "parakeet-local"}
            available={pa().available}
            unavailableReason={pa().reason}
            note="Custom dictionary biasing isn't supported on Parakeet in this version. Switch to Whisper if you rely on the Dictionary tab for technical vocabulary."
          >
            <For each={parakeetModels()}>
              {(m) => (
                <ModelRow
                  m={m}
                  isActive={
                    props.s.stt_engine === "parakeet-local"
                    && props.s.stt_model_id === m.descriptor.id
                  }
                  progress={progress()[m.descriptor.id]}
                  downloading={downloading().has(m.descriptor.id)}
                  onUse={() =>
                    setActiveStt("parakeet-local", m.descriptor.id)
                  }
                  onDownload={() => startDownload(m.descriptor.id)}
                  onCancel={() => cancel(m.descriptor.id)}
                  onDelete={() => remove(m.descriptor.id)}
                />
              )}
            </For>
          </SttEngineCard>
        )}
      </Show>

      <SttEngineCard
        title="Whisper (OpenAI)"
        subtitle="OpenAI Whisper — universal multilingual coverage, broader language support."
        recommendation="Recommended for languages outside Parakeet's 25-language set."
        engineId="whisper-local"
        isActive={props.s.stt_engine === "whisper-local"}
        available={true}
        unavailableReason={null}
        note={null}
      >
        <For each={whisperModels()}>
          {(m) => (
            <ModelRow
              m={m}
              isActive={
                props.s.stt_engine === "whisper-local"
                && props.s.stt_model_id === m.descriptor.id
              }
              progress={progress()[m.descriptor.id]}
              downloading={downloading().has(m.descriptor.id)}
              onUse={() => setActiveStt("whisper-local", m.descriptor.id)}
              onDownload={() => startDownload(m.descriptor.id)}
              onCancel={() => cancel(m.descriptor.id)}
              onDelete={() => remove(m.descriptor.id)}
            />
          )}
        </For>
      </SttEngineCard>

      <SectionHeader
        title="Transcript polishing"
        hint={props.s.polishing_enabled ? "Local · Gemma" : "Off"}
      />
      <Card>
        <Row
          label="Clean up each transcript"
          hint="Fix punctuation, drop filler words, and apply the active Voice Mode."
          last={
            !(props.s.polishing_enabled && props.s.llm_engine === "gemma-local")
          }
        >
          <Toggle
            checked={props.s.polishing_enabled}
            onChange={(v) => setPolishing(v)}
          />
        </Row>
        <Show
          when={
            props.s.polishing_enabled && props.s.llm_engine === "gemma-local"
          }
        >
          <Row
            label="Thinking"
            hint="Let Gemma reason silently before polishing. Slower, but can improve grammar on tricky inputs. Off by default."
            last
          >
            <Toggle
              checked={props.s.enable_thinking}
              onChange={(v) => props.update({ enable_thinking: v })}
            />
          </Row>
        </Show>
      </Card>
      <Show when={props.s.polishing_enabled}>
        <SectionHeader
          title="Polishing engine"
          hint={
            props.s.llm_engine === "apple-fm"
              ? "Apple Intelligence"
              : "Gemma 4 (local)"
          }
        />
        <Show when={appleFm()?.supported_host}>
          <AppleFmEngineRow
            availability={appleFm()}
            isActive={props.s.llm_engine === "apple-fm"}
            onUse={setEngineAppleFm}
            onRefresh={refreshAppleFm}
          />
        </Show>
        <GemmaEngineRow
          isActive={props.s.llm_engine === "gemma-local"}
          onUse={setEngineGemma}
        />
        <Show when={props.s.llm_engine === "gemma-local"}>
          <Show when={!props.s.gemma_tos_accepted}>
            <div class="mb-3 rounded-[10px] bg-[var(--color-accent-soft)] p-3 ring-1 ring-[var(--color-accent)]/40 text-[12px] text-[var(--color-ink)]">
              You'll be asked to accept the{" "}
              <a
                class="underline"
                href="https://ai.google.dev/gemma/terms"
                onClick={(e) => {
                  e.preventDefault();
                  void openExternalUrl("https://ai.google.dev/gemma/terms");
                }}
              >
                Gemma Terms of Use
              </a>{" "}
              on first download.
            </div>
          </Show>
          <InferenceServerSection />
          <SectionHeader
            title="Polishing model"
            hint={activeLlm()?.descriptor.tier ? `${activeLlm()!.descriptor.tier} tier` : undefined}
          />
          <For each={llmModels()}>
            {(m) => (
              <ModelRow
                m={m}
                isActive={
                  props.s.llm_engine === "gemma-local" &&
                  props.s.llm_model_id === m.descriptor.id
                }
                progress={progress()[m.descriptor.id]}
                downloading={downloading().has(m.descriptor.id)}
                onUse={() => setActiveLlm(m.descriptor.id)}
                onDownload={() => startDownload(m.descriptor.id)}
                onCancel={() => cancel(m.descriptor.id)}
                onDelete={() => remove(m.descriptor.id)}
              />
            )}
          </For>
        </Show>
      </Show>
    </>
  );
}
