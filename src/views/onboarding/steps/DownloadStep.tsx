import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import {
  acceptGemmaTos,
  appleFmAvailability,
  downloadInferenceServer,
  downloadModel,
  inferenceServerStatus,
  listModels,
  loadModelWarm,
  onBinaryDone,
  onBinaryError,
  onBinaryProgress,
  onModelDone,
  onModelError,
  onModelProgress,
  openExternalUrl,
  type AppleFmAvailability,
  type InferenceServerStatus,
  type ModelStatus,
  type Settings,
  type TierRecommendation,
} from "@/lib/ipc";
import { DownloadBar } from "../components/DownloadBar";
import { ModelCard } from "../components/ModelCard";
import { PolishEngineCard } from "../components/PolishEngineCard";
import { PrewarmCard } from "../components/PrewarmCard";
import {
  ETA_MAX_SAMPLES,
  ETA_WINDOW_MS,
  type PolishChoice,
  type ProgressSample,
} from "../constants";
import { shortModelName } from "../helpers";

export function DownloadStep(props: {
  s: Settings | null;
  tier: TierRecommendation | null;
  patch: (p: Partial<Settings>) => Promise<void>;
  /// Parent owns the Continue gate; this step owns the IDs being downloaded.
  onInFlightChange: (v: boolean) => void;
  /// Tells the parent when every required artifact for the chosen
  /// engine is on disk. Continue stays disabled until this flips true.
  onReadyChange: (v: boolean) => void;
}) {
  const [progress, setProgress] = createSignal<
    Record<string, { downloaded: number; total: number }>
  >({});
  /// Toggled after every download bar reads "Done" and stays on until
  /// the engine pre-warm finishes. Drives the visible indicator under
  /// the download bars so the 15-30 s silent wait between "Done" and
  /// Continue-enabled stops looking like the app is hung.
  const [prewarmInFlight, setPrewarmInFlight] = createSignal(false);
  // Per-id rolling samples. Stored as a flat Record (not nested in
  // `progress`) so a ResizeObserver-style spread doesn't drop the
  // history on each tick.
  const [samples, setSamples] = createSignal<Record<string, ProgressSample[]>>(
    {},
  );
  const [completed, setCompleted] = createSignal<Set<string>>(new Set());
  const [started, setStarted] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);
  // Wall-clock timestamp of when the user kicked off the downloads.
  // Combined with the rolling `samples` history to detect a download
  // that never produced a single progress event (the VPN/firewall
  // case) — distinct from one that started fine and then stalled.
  const [startedAt, setStartedAt] = createSignal<number | null>(null);
  // 1 Hz ticker that drives the stall check. Stays inert until
  // `started()` flips on; cleaned up when the step unmounts.
  const [nowMs, setNowMs] = createSignal(Date.now());
  const [sttModels, setSttModels] = createSignal<ModelStatus[]>([]);
  const [llmModels, setLlmModels] = createSignal<ModelStatus[]>([]);
  const [selectedSttId, setSelectedSttId] = createSignal<string | null>(null);
  const [selectedLlmId, setSelectedLlmId] = createSignal<string | null>(null);
  const [binaryStatus, setBinaryStatus] =
    createSignal<InferenceServerStatus | null>(null);
  const [appleFm, setAppleFm] = createSignal<AppleFmAvailability | null>(null);
  // Always default to the local Gemma stack: Apple Intelligence is
  // faster but produces lower-quality polishing in our testing, so we
  // surface it as a tradeoff rather than the default.
  const [polishChoice, setPolishChoice] = createSignal<PolishChoice>(
    "gemma-local",
  );
  const unlistens: (() => void)[] = [];

  onMount(async () => {
    // model:* and binary:* emit the same payload shape, so handlers
    // share the progress/completed maps; per-id keys disambiguate.
    const handleProgress = (p: {
      id: string;
      bytes_downloaded: number;
      total_bytes: number;
    }) => {
      setProgress((prev) => ({
        ...prev,
        [p.id]: { downloaded: p.bytes_downloaded, total: p.total_bytes },
      }));
      const now = Date.now();
      setSamples((prev) => {
        const history = prev[p.id] ?? [];
        // Drop samples older than the smoothing window. The 200 ms
        // emit cadence from `models::stream_to_file` yields ~25
        // samples per 5 s, but a slow link can pace much slower —
        // keep the cap at ETA_MAX_SAMPLES regardless so we always
        // average over a few points instead of just one.
        const cutoff = now - ETA_WINDOW_MS;
        const trimmed = history
          .filter((s) => s.ts >= cutoff)
          .slice(-(ETA_MAX_SAMPLES - 1));
        trimmed.push({ ts: now, bytes: p.bytes_downloaded });
        return { ...prev, [p.id]: trimmed };
      });
    };
    const handleDone = (p: { id: string }) => {
      setCompleted((prev) => new Set(prev).add(p.id));
    };
    const handleError = (p: { id: string; error: string }) => {
      setErr(p.error);
    };

    unlistens.push(await onModelProgress(handleProgress));
    unlistens.push(await onModelDone(handleDone));
    unlistens.push(await onModelError(handleError));
    unlistens.push(await onBinaryProgress(handleProgress));
    unlistens.push(await onBinaryDone(handleDone));
    unlistens.push(await onBinaryError(handleError));

    try {
      const [all, bin, fm] = await Promise.all([
        listModels(),
        inferenceServerStatus(),
        appleFmAvailability(),
      ]);
      setSttModels(all.filter((m) => m.descriptor.kind === "stt"));
      setLlmModels(all.filter((m) => m.descriptor.kind === "llm"));
      setBinaryStatus(bin);
      setAppleFm(fm);
      // Re-runs via "Replay onboarding": pre-mark anything already
      // on disk so the requirements gate doesn't block forever and
      // returning users are not asked to re-download.
      setCompleted((prev) => {
        const next = new Set(prev);
        for (const m of all) {
          if (m.installed) next.add(m.descriptor.id);
        }
        if (bin.installed) next.add(bin.id);
        return next;
      });
    } catch (e) {
      console.warn("onboarding probes failed", e);
    }
  });

  /// Re-probe the filesystem and authoritatively update `completed`
  /// with everything actually on disk. Used as a safety net after the
  /// `Promise.all(downloads…)` resolves — the `model:done`/`binary:done`
  /// events flow over a separate IPC channel from the command response,
  /// and a slow listener can leave the readiness memo stale even after
  /// `download_model` returned Ok. Probing disk directly is the source
  /// of truth.
  async function reconcileCompletedFromDisk() {
    try {
      const [all, bin] = await Promise.all([
        listModels(),
        inferenceServerStatus(),
      ]);
      setSttModels(all.filter((m) => m.descriptor.kind === "stt"));
      setLlmModels(all.filter((m) => m.descriptor.kind === "llm"));
      setBinaryStatus(bin);
      setCompleted((prev) => {
        const next = new Set(prev);
        for (const m of all) {
          if (m.installed) next.add(m.descriptor.id);
        }
        if (bin.installed) next.add(bin.id);
        return next;
      });
    } catch (e) {
      console.warn("post-download disk reconcile failed", e);
    }
  }

  // Filter the STT options shown in the form to the engine the user
  // picked in the Language step — there's no point in offering Whisper
  // sizes when they're going to download Parakeet, and vice versa.
  // Falls back to "all STT models" before settings are loaded.
  const visibleSttModels = createMemo(() => {
    const all = sttModels();
    const eng = props.s?.stt_engine;
    if (!eng) return all;
    return all.filter((m) => m.descriptor.engine === eng);
  });

  // Auto-select default models. Prefer the user's persisted setting if
  // it still maps to a catalog entry (Replay onboarding), else the
  // hardware-tier recommendation. Runs once each — `selectedXId()` gate
  // makes the effect idempotent.
  createEffect(() => {
    const t = props.tier;
    const stts = visibleSttModels();
    const cur = props.s;
    if (!t || stts.length === 0 || selectedSttId()) return;
    const stored = cur?.stt_model_id;
    const ok = stored && stts.some((m) => m.descriptor.id === stored);
    setSelectedSttId(ok ? stored! : stts[0].descriptor.id);
  });

  createEffect(() => {
    const t = props.tier;
    const llms = llmModels();
    const cur = props.s;
    if (!t || llms.length === 0 || selectedLlmId()) return;
    const stored = cur?.llm_model_id;
    const ok = stored && llms.some((m) => m.descriptor.id === stored);
    setSelectedLlmId(ok ? stored! : t.llm_model_id);
  });

  // Compute "all required artifacts on disk" for the chosen engine and
  // forward it to the parent's Continue button. The user must click
  // the action button at least once (`started()`) so settings are
  // committed for their current selection — otherwise a returning
  // user could advance with stale `llm_engine` from a prior session.
  const requirementsMet = createMemo(() => {
    if (!started()) return false;
    const sttId = selectedSttId();
    const choice = polishChoice();
    const inst = completed();
    if (!sttId || !inst.has(sttId)) return false;
    if (choice === "apple-fm") return true;
    const llmId = selectedLlmId();
    const bin = binaryStatus();
    if (!llmId || !bin) return false;
    return inst.has(llmId) && inst.has(bin.id);
  });

  createEffect(() => {
    props.onReadyChange(requirementsMet());
  });

  // Drive the stall-hint clock only while a session of downloads is
  // actually in flight, so an idle wizard tab isn't waking the event
  // loop every second.
  createEffect(() => {
    if (!started()) return;
    if (startedAt() === null) setStartedAt(Date.now());
    const tick = window.setInterval(() => setNowMs(Date.now()), 1000);
    onCleanup(() => window.clearInterval(tick));
  });

  // IDs of artifacts the user's current engine choice expects to
  // pull down. Used to single out which downloads "should be moving"
  // when checking for a stall.
  const expectedIds = createMemo(() => {
    const ids: string[] = [];
    const sId = selectedSttId();
    if (sId) ids.push(sId);
    if (polishChoice() === "gemma-local") {
      const lId = selectedLlmId();
      if (lId) ids.push(lId);
      const b = binaryStatus();
      if (b) ids.push(b.id);
    }
    return ids;
  });

  // A download is "stuck" if it's been > STALL_WINDOW_MS since the
  // last progress event for any artifact still pending — or, for the
  // worst case where no event has ever arrived, since `started()`
  // flipped on. The classic VPN failure mode is the second one: the
  // request never gets past TLS, so 0 bytes ever land.
  //
  // Suppressed while an error is already on screen so we don't stack
  // two notes; the error paragraph carries the same diagnostic weight.
  const STALL_WINDOW_MS = 15_000;
  const showStalledHint = createMemo(() => {
    if (!started()) return false;
    if (err()) return false;
    const startTs = startedAt();
    if (startTs === null) return false;
    const done = completed();
    const samp = samples();
    const tNow = nowMs();
    for (const id of expectedIds()) {
      if (done.has(id)) continue;
      const hist = samp[id];
      const lastTs = hist && hist.length > 0
        ? hist[hist.length - 1].ts
        : startTs;
      if (tNow - lastTs > STALL_WINDOW_MS) return true;
    }
    return false;
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
    // Stop pretending the gate is met when this step unmounts —
    // otherwise a back-nav into the mic step would still advance.
    props.onReadyChange(false);
  });

  const selectedSttDescriptor = () => {
    const id = selectedSttId();
    if (!id) return null;
    return sttModels().find((m) => m.descriptor.id === id)?.descriptor ?? null;
  };
  const selectedLlmDescriptor = () => {
    const id = selectedLlmId();
    if (!id) return null;
    return llmModels().find((m) => m.descriptor.id === id)?.descriptor ?? null;
  };

  async function start() {
    const t = props.tier;
    const cur = props.s;
    const sttId = selectedSttId();
    const choice = polishChoice();
    if (!t || !cur || !sttId) return;
    setStarted(true);

    // Derive the engine id from the chosen model descriptor — the
    // user could have switched engines via the Language step but the
    // settings store hasn't echoed yet.
    const sttEngine =
      sttModels().find((m) => m.descriptor.id === sttId)?.descriptor.engine
        ?? cur.stt_engine
        ?? "whisper-local";

    if (choice === "apple-fm") {
      // Apple Intelligence: no GGUF download, no llama-server. STT
      // is still local and still needs to come down.
      await props.patch({
        stt_engine: sttEngine,
        stt_model_id: sttId,
        llm_engine: "apple-fm",
        // Keep the recommended Gemma id so a later switch in Settings
        // doesn't have to re-pick the tier-matched one. Honour the
        // user's prior pick if they had one; else fall back to tier.
        llm_model_id: selectedLlmId() ?? t.llm_model_id,
        polishing_enabled: true,
      });
      try {
        props.onInFlightChange(true);
        await downloadModel(sttId);
        // Reconcile against disk before pre-warm — the `model:done`
        // event arrives on a separate IPC channel from the command
        // response, and we don't want Continue to wait on a slow event
        // when the bytes are already on disk.
        await reconcileCompletedFromDisk();
        setPrewarmInFlight(true);
        try {
          await loadModelWarm(sttId);
        } finally {
          setPrewarmInFlight(false);
        }
      } catch (e) {
        setErr(String(e));
      } finally {
        props.onInFlightChange(false);
      }
      return;
    }

    const llmId = selectedLlmId();
    if (!llmId) {
      setStarted(false);
      return;
    }
    if (!cur.gemma_tos_accepted) {
      await acceptGemmaTos();
    }
    await props.patch({
      stt_engine: sttEngine,
      stt_model_id: sttId,
      llm_engine: "gemma-local",
      llm_model_id: llmId,
      gemma_tos_accepted: true,
      polishing_enabled: true,
    });
    try {
      props.onInFlightChange(true);
      await Promise.all([
        downloadModel(sttId),
        downloadModel(llmId),
        downloadInferenceServer(),
      ]);
      // Reconcile against disk before pre-warm. Without this, a
      // `model:done` or `binary:done` event that arrives slowly leaves
      // `completed` stale, and the Continue gate stays disabled even
      // though every artifact is on disk — that was the original "must
      // click Back then Next" bug. Reading from `list_models` /
      // `inference_server_status` is the source of truth.
      await reconcileCompletedFromDisk();
      // Pre-warm so first dictation doesn't pay the cold-load tax. The
      // inference server is deferred until first polish — spawning it
      // now would burn RAM before the user even dictates.
      setPrewarmInFlight(true);
      try {
        await Promise.allSettled([
          loadModelWarm(sttId),
          loadModelWarm(llmId),
        ]);
      } finally {
        setPrewarmInFlight(false);
      }
    } catch (e) {
      setErr(String(e));
    } finally {
      props.onInFlightChange(false);
    }
  }

  // Disable the action button until the user has made the necessary
  // selections. Once clicked, `started()` swaps the form for the
  // progress bars; the top-right Continue button takes over from there.
  const startDisabled = () =>
    !props.tier ||
    !selectedSttId() ||
    (polishChoice() === "gemma-local" && !selectedLlmId());

  // The wrapping <main> owns the scrollbar. Setting overflow-y here
  // would also activate horizontal clipping (browsers compute
  // overflow-x: auto when overflow-y is set with default x), which
  // pinches the selected card's `ring-2` accent at the box edge.
  return (
    <div class="flex w-full max-w-[440px] flex-col gap-4">
      <h2 class="text-[20px] font-semibold">Set up your dictation models.</h2>
      <Show when={!started()}>
        <p class="text-[13px] text-[var(--color-ink-mute)]">
          Vox runs everything on this Mac. Pick a transcription model and
          a polishing engine to clean each transcript up.
        </p>

        <Show when={visibleSttModels().length > 0 && props.tier}>
          {(t) => (
            <div
              class="flex flex-col gap-2"
              role="radiogroup"
              aria-label="Speech recognition model"
            >
              <div class="flex items-baseline justify-between">
                <span class="text-[13px] font-medium">
                  Speech recognition (
                  {props.s?.stt_engine === "parakeet-local"
                    ? "Parakeet"
                    : "Whisper"}
                  )
                </span>
                <Show when={t().physical_ram_gb > 0}>
                  <span class="text-[11px] text-[var(--color-ink-faint)]">
                    Your Mac has {t().physical_ram_gb} GB RAM
                  </span>
                </Show>
              </div>
              <For each={visibleSttModels()}>
                {(m) => (
                  <ModelCard
                    model={m}
                    selected={selectedSttId() === m.descriptor.id}
                    recommended={
                      m.descriptor.engine === "parakeet-local"
                        || m.descriptor.id === t().stt_model_id
                    }
                    swapWarn={
                      t().physical_ram_gb > 0 &&
                      m.descriptor.ram_peak_mb >
                        t().physical_ram_gb * 1024 * 0.85
                    }
                    onSelect={() => setSelectedSttId(m.descriptor.id)}
                  />
                )}
              </For>
            </div>
          )}
        </Show>

        <div
          class="flex flex-col gap-2"
          role="radiogroup"
          aria-label="Polishing engine"
        >
          <span class="text-[13px] font-medium">Transcript polishing</span>
          <PolishEngineCard
            engine="gemma-local"
            title="Gemma 4 (open-source)"
            subtitle="Runs entirely offline. Choose a size below."
            recommended={true}
            note={null}
            disabled={false}
            disabledReason={null}
            selected={polishChoice() === "gemma-local"}
            onSelect={() => setPolishChoice("gemma-local")}
          />
          <PolishEngineCard
            engine="apple-fm"
            title="Apple Intelligence"
            subtitle="On-device. No download. Requires macOS 26 + Apple Silicon."
            recommended={false}
            // Surfaces the quality tradeoff so the user picks the
            // tradeoff knowingly: Apple's Foundation Models polish
            // faster but produce lower-quality cleanups in our tests.
            note="Faster, but lower polishing quality."
            disabled={!(appleFm()?.available ?? false)}
            disabledReason={appleFm()?.message ?? null}
            selected={polishChoice() === "apple-fm"}
            onSelect={() => setPolishChoice("apple-fm")}
          />
        </div>

        <Show when={polishChoice() === "gemma-local" && llmModels().length > 0 && props.tier}>
          {(t) => (
            <div
              class="flex flex-col gap-2"
              role="radiogroup"
              aria-label="Gemma 4 model"
            >
              <span class="text-[13px] font-medium">Gemma 4 model</span>
              <For each={llmModels()}>
                {(m) => (
                  <ModelCard
                    model={m}
                    selected={selectedLlmId() === m.descriptor.id}
                    recommended={m.descriptor.id === t().llm_model_id}
                    swapWarn={
                      t().physical_ram_gb > 0 &&
                      m.descriptor.ram_peak_mb >
                        t().physical_ram_gb * 1024 * 0.85
                    }
                    onSelect={() => setSelectedLlmId(m.descriptor.id)}
                  />
                )}
              </For>
              <p class="text-[11px] text-[var(--color-ink-faint)]">
                By continuing you accept the{" "}
                <a
                  class="underline"
                  href="https://ai.google.dev/gemma/terms"
                  onClick={(e) => {
                    e.preventDefault();
                    void openExternalUrl("https://ai.google.dev/gemma/terms");
                  }}
                >
                  Gemma Terms of Use
                </a>
                .
              </p>
            </div>
          )}
        </Show>

        <button
          type="button"
          class="self-start rounded-[8px] bg-[var(--color-accent)] px-4 py-2 text-[13px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={startDisabled()}
          onClick={start}
        >
          {polishChoice() === "apple-fm"
            ? "Continue with Apple Intelligence"
            : "Accept and download"}
        </button>
      </Show>
      <Show when={started()}>
        <div class="space-y-3">
          <Show when={selectedSttDescriptor()}>
            {(d) => (
              <DownloadBar
                label={shortModelName(d().name)}
                done={completed().has(d().id)}
                prog={progress()[d().id]}
                samples={samples()[d().id]}
              />
            )}
          </Show>
          <Show when={polishChoice() === "gemma-local" && selectedLlmDescriptor()} fallback={null}>
            {(d) => (
              <DownloadBar
                label={shortModelName(d().name)}
                done={completed().has(d().id)}
                prog={progress()[d().id]}
                samples={samples()[d().id]}
              />
            )}
          </Show>
          <Show when={polishChoice() === "gemma-local" && binaryStatus()}>
            {(b) => (
              <DownloadBar
                label="Inference engine"
                done={completed().has(b().id)}
                prog={progress()[b().id]}
                samples={samples()[b().id]}
              />
            )}
          </Show>
          <Show when={prewarmInFlight()}>
            <PrewarmCard />
          </Show>
          <Show when={showStalledHint()}>
            <div class="flex flex-col gap-1 text-[11px] leading-[1.45] text-[var(--color-ink-mute)]">
              <p>
                Slow or stuck? A VPN, firewall, or proxy can block downloads
                from Hugging Face and GitHub.
              </p>
              <p>
                Still stuck? Ask in the{" "}
                <a
                  class="underline"
                  href="https://discord.gg/W5nDZFeY"
                  onClick={(e) => {
                    e.preventDefault();
                    void openExternalUrl("https://discord.gg/W5nDZFeY");
                  }}
                >
                  Vox Discord
                </a>
                .
              </p>
            </div>
          </Show>
        </div>
      </Show>
      <Show when={err()}>
        <p class="text-[12px] text-[var(--color-danger)]">{err()}</p>
      </Show>
    </div>
  );
}
