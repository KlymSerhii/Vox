import { createSignal, Show, onMount, onCleanup } from "solid-js";
import {
  acceptGemmaTos,
  dismissMigration,
  downloadModel,
  getSettings,
  onModelDone,
  onModelError,
  onModelProgress,
  recommendTier,
  setSettings,
  type Settings,
  type TierRecommendation,
} from "../lib/ipc";

type Phase = "intro" | "tos" | "downloading" | "done";

export default function MigrationView() {
  const [phase, setPhase] = createSignal<Phase>("intro");
  const [s, setS] = createSignal<Settings | null>(null);
  const [tier, setTier] = createSignal<TierRecommendation | null>(null);
  const [progress, setProgress] = createSignal<
    Record<string, { downloaded: number; total: number }>
  >({});
  const [completed, setCompleted] = createSignal<Set<string>>(new Set());
  const [err, setErr] = createSignal<string | null>(null);

  const unlistens: (() => void)[] = [];

  onMount(async () => {
    setS(await getSettings());
    setTier(await recommendTier());
    unlistens.push(
      await onModelProgress((p) => {
        setProgress((prev) => ({
          ...prev,
          [p.id]: { downloaded: p.bytes_downloaded, total: p.total_bytes },
        }));
      }),
    );
    unlistens.push(
      await onModelDone((p) => {
        setCompleted((prev) => {
          const n = new Set(prev);
          n.add(p.id);
          return n;
        });
      }),
    );
    unlistens.push(
      await onModelError((p) => {
        setErr(p.error);
      }),
    );
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
  });

  async function keepCloud() {
    const cur = s();
    if (!cur) return;
    const next = {
      ...cur,
      stt_engine: "groq",
      llm_engine: cur.anthropic_api_key ? "anthropic" : "none",
      migration_seen: true,
    };
    await setSettings(next);
    await dismissMigration();
  }

  async function switchToLocal() {
    const cur = s();
    const t = tier();
    if (!cur || !t) return;
    if (!cur.gemma_tos_accepted) {
      setPhase("tos");
      return;
    }
    await beginDownloads();
  }

  async function acceptTosAndDownload() {
    await acceptGemmaTos();
    const cur = await getSettings();
    setS(cur);
    await beginDownloads();
  }

  async function beginDownloads() {
    const cur = s();
    const t = tier();
    if (!cur || !t) return;
    const next: Settings = {
      ...cur,
      stt_engine: "whisper-local",
      stt_model_id: t.stt_model_id,
      llm_engine: "gemma-local",
      llm_model_id: t.llm_model_id,
      migration_seen: true,
    };
    await setSettings(next);
    setPhase("downloading");
    try {
      await Promise.all([downloadModel(t.stt_model_id), downloadModel(t.llm_model_id)]);
    } catch (e) {
      setErr(String(e));
    }
    setPhase("done");
  }

  async function finish() {
    await dismissMigration();
  }

  return (
    <div class="flex h-screen w-screen flex-col bg-[var(--color-bg)] text-[var(--color-ink)] p-6">
      <Show when={phase() === "intro"}>
        <div class="flex flex-1 flex-col items-center justify-center gap-5 text-center">
          <h1 class="text-[22px] font-semibold">Vox now runs offline.</h1>
          <p class="max-w-[420px] text-[13px] text-[var(--color-ink-mute)]">
            Switch to local models to keep your audio and transcripts on your
            Mac. Cloud still works if you prefer it.
          </p>
          <Show when={tier()}>
            {(t) => (
              <div class="rounded-[8px] bg-[var(--color-surface-raised)] p-3 text-[12px] text-[var(--color-ink-mute)] ring-1 ring-[var(--color-border)]">
                Detected: {t().physical_ram_gb} GB RAM · recommending the{" "}
                <span class="text-[var(--color-ink)]">{t().tier}</span> tier.
                <Show when={t().low_ram_warning}>
                  <div class="mt-1 text-[var(--color-accent)]">
                    Low RAM — Vox will recommend Whisper Small.
                  </div>
                </Show>
              </div>
            )}
          </Show>
          <div class="mt-2 flex gap-2">
            <button
              type="button"
              class="rounded-[8px] bg-white/8 px-4 py-2 text-[13px] hover:bg-white/12"
              onClick={keepCloud}
            >
              Keep using cloud
            </button>
            <button
              type="button"
              class="rounded-[8px] bg-[var(--color-accent)] px-4 py-2 text-[13px] font-medium text-black hover:brightness-110"
              onClick={switchToLocal}
            >
              Switch to local models
            </button>
          </div>
        </div>
      </Show>

      <Show when={phase() === "tos"}>
        <div class="flex flex-1 flex-col items-center justify-center gap-5 text-center">
          <h1 class="text-[20px] font-semibold">Gemma Terms of Use</h1>
          <p class="max-w-[420px] text-[13px] text-[var(--color-ink-mute)]">
            Vox uses Google's Gemma 4 model for cleanup. You'll download it
            from Hugging Face. Please review the{" "}
            <a
              class="underline"
              href="https://ai.google.dev/gemma/terms"
              target="_blank"
            >
              Gemma Terms of Use
            </a>{" "}
            before continuing.
          </p>
          <div class="flex gap-2">
            <button
              type="button"
              class="rounded-[8px] bg-white/8 px-4 py-2 text-[13px] hover:bg-white/12"
              onClick={() => setPhase("intro")}
            >
              Back
            </button>
            <button
              type="button"
              class="rounded-[8px] bg-[var(--color-accent)] px-4 py-2 text-[13px] font-medium text-black hover:brightness-110"
              onClick={acceptTosAndDownload}
            >
              Accept and continue
            </button>
          </div>
        </div>
      </Show>

      <Show when={phase() === "downloading"}>
        <div class="flex flex-1 flex-col items-center justify-center gap-5 text-center">
          <h1 class="text-[20px] font-semibold">Downloading models…</h1>
          <Show when={tier()}>
            {(t) => (
              <div class="w-full max-w-[400px] space-y-3">
                <DownloadBar
                  label="Whisper"
                  done={completed().has(t().stt_model_id)}
                  prog={progress()[t().stt_model_id]}
                />
                <DownloadBar
                  label="Gemma"
                  done={completed().has(t().llm_model_id)}
                  prog={progress()[t().llm_model_id]}
                />
              </div>
            )}
          </Show>
          <Show when={err()}>
            <p class="text-[12px] text-[var(--color-danger)]">{err()}</p>
          </Show>
        </div>
      </Show>

      <Show when={phase() === "done"}>
        <div class="flex flex-1 flex-col items-center justify-center gap-5 text-center">
          <h1 class="text-[20px] font-semibold">All set.</h1>
          <p class="max-w-[420px] text-[13px] text-[var(--color-ink-mute)]">
            Vox is now running offline. Hold your hotkey and dictate as
            usual.
          </p>
          <button
            type="button"
            class="rounded-[8px] bg-[var(--color-accent)] px-4 py-2 text-[13px] font-medium text-black hover:brightness-110"
            onClick={finish}
          >
            Done
          </button>
        </div>
      </Show>
    </div>
  );
}

function DownloadBar(props: {
  label: string;
  done: boolean;
  prog?: { downloaded: number; total: number };
}) {
  const pct = () => {
    if (props.done) return 100;
    if (!props.prog) return 0;
    if (props.prog.total <= 0) return 0;
    return Math.min(100, (props.prog.downloaded / props.prog.total) * 100);
  };
  return (
    <div>
      <div class="mb-1 flex justify-between text-[11px] text-[var(--color-ink-mute)]">
        <span>{props.label}</span>
        <span>{props.done ? "Done" : `${pct().toFixed(0)}%`}</span>
      </div>
      <div class="h-1.5 w-full overflow-hidden rounded-full bg-white/8">
        <div
          class="h-full rounded-full bg-[var(--color-accent)] transition-[width]"
          style={{ width: `${pct()}%` }}
        />
      </div>
    </div>
  );
}
