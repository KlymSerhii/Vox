import { createSignal, onCleanup, onMount, Show } from "solid-js";
import {
  cancelInferenceServerDownload,
  deleteInferenceServer,
  downloadInferenceServer,
  inferenceServerStatus,
  onBinaryCanceled,
  onBinaryDone,
  onBinaryError,
  onBinaryProgress,
  restartInferenceServer,
  type InferenceServerStatus,
} from "@/lib/ipc";
import { Card } from "../components/Card";
import { Row } from "../components/Row";
import { SectionHeader } from "../components/SectionHeader";
import { formatBytes } from "../helpers";
import { useConfirm } from "../useConfirm";

// Sits above the cleanup-models list because the binary is the
// prerequisite — without it, GGUFs on disk do nothing. Polish requests
// fail-open in the orchestrator, so this row exists for the user
// trying to figure out *why* polish stopped working.
export function InferenceServerSection() {
  const [status, setStatus] = createSignal<InferenceServerStatus | null>(null);
  const [progress, setProgress] = createSignal<{
    downloaded: number;
    total: number;
    eta: number | null;
  } | null>(null);
  const [downloading, setDownloading] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);

  let unlistens: (() => void)[] = [];

  async function refresh() {
    try {
      setStatus(await inferenceServerStatus());
    } catch (e) {
      console.error("inferenceServerStatus:", e);
    }
  }

  onMount(async () => {
    await refresh();
    unlistens.push(
      await onBinaryProgress((p) => {
        setProgress({
          downloaded: p.bytes_downloaded,
          total: p.total_bytes,
          eta: p.eta_secs,
        });
      }),
    );
    unlistens.push(
      await onBinaryDone(async () => {
        setDownloading(false);
        setProgress(null);
        setErr(null);
        await refresh();
      }),
    );
    unlistens.push(
      await onBinaryError((p) => {
        setErr(p.error);
        setDownloading(false);
      }),
    );
    unlistens.push(
      await onBinaryCanceled(() => {
        setDownloading(false);
        setProgress(null);
      }),
    );
  });

  onCleanup(() => {
    unlistens.forEach((u) => u());
  });

  async function install() {
    setDownloading(true);
    setErr(null);
    try {
      await downloadInferenceServer();
    } catch (e) {
      setErr(String(e));
      setDownloading(false);
    }
  }

  // Two-step confirm. `window.confirm` is a silent no-op in Tauri's WKWebView
  // so we render a Confirm/Cancel pair on the row instead.
  const confirmReinstall = useConfirm();
  async function reinstall() {
    try {
      await deleteInferenceServer();
      await refresh();
      await install();
    } catch (e) {
      console.error("reinstall:", e);
      setErr(String(e));
    }
  }

  async function restart() {
    try {
      await restartInferenceServer();
    } catch (e) {
      console.error("restart inference:", e);
    }
  }

  const supported = () => status()?.supported_host ?? false;
  const installed = () => status()?.installed ?? false;
  const version = () => status()?.version ?? "";

  return (
    <Show when={supported()}>
      <SectionHeader
        title="Inference engine"
        hint={installed() ? "Installed" : "Required"}
      />
      <Card>
        <Row
          label={
            version()
              ? `llama-server (llama.cpp build ${version()})`
              : "llama-server"
          }
          hint={
            installed()
              ? "Pre-built, sandboxed inference subprocess. Crashes here can't take down Vox."
              : "Pre-built inference subprocess. Required for transcript polishing."
          }
          last
        >
          <div class="flex items-center gap-2">
            <Show when={!installed() && !downloading()}>
              <button
                type="button"
                class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:brightness-110"
                onClick={install}
              >
                Install
              </button>
            </Show>
            <Show when={installed() && !downloading() && !confirmReinstall.armed()}>
              <button
                type="button"
                class="rounded-[6px] px-3 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
                onClick={restart}
                title="Stop the running server. The next polish will spawn a fresh one."
              >
                Restart
              </button>
              <button
                type="button"
                class="rounded-[6px] px-3 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
                onClick={() => confirmReinstall.armOrFire(() => void reinstall())}
              >
                Reinstall
              </button>
            </Show>
            <Show when={installed() && !downloading() && confirmReinstall.armed()}>
              <span class="text-[11px] text-[var(--color-ink-mute)]">
                Reinstall? Re-downloads the binary.
              </span>
              <button
                type="button"
                class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] hover:bg-[var(--color-control-hover)]"
                onClick={confirmReinstall.disarm}
              >
                Cancel
              </button>
              <button
                type="button"
                class="rounded-[6px] bg-[var(--color-danger)] px-3 py-1 text-[11px] font-medium text-white hover:brightness-110"
                onClick={() => confirmReinstall.armOrFire(() => void reinstall())}
              >
                Reinstall
              </button>
            </Show>
            <Show when={downloading()}>
              <button
                type="button"
                class="rounded-[6px] px-3 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
                onClick={() => cancelInferenceServerDownload()}
              >
                Cancel
              </button>
            </Show>
          </div>
        </Row>
        <Show when={progress()}>
          {(p) => (
            <div class="px-3 pb-3">
              <div class="flex justify-between text-[11px] text-[var(--color-ink-mute)]">
                <span>
                  {formatBytes(p().downloaded)} / {formatBytes(p().total)}
                </span>
                <Show when={p().eta != null}>
                  <span>ETA {p().eta}s</span>
                </Show>
              </div>
              <div class="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-[var(--color-control)]">
                <div
                  class="h-full rounded-full bg-[var(--color-accent)]"
                  style={{
                    width: `${p().total > 0 ? Math.min(100, (p().downloaded / p().total) * 100) : 0}%`,
                  }}
                />
              </div>
            </div>
          )}
        </Show>
        <Show when={err()}>
          <p class="px-3 pb-3 text-[11px] text-[var(--color-danger)]">
            {err()}
          </p>
        </Show>
      </Card>
    </Show>
  );
}
