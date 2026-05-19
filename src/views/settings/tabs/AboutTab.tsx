import { createResource, createSignal, onCleanup, onMount, Show } from "solid-js";
import { VoxLogo } from "@/components/VoxLogo";
import { SparkleGlyph } from "@/components/SparkleGlyph";
import {
  checkForUpdatesNow,
  diagnoseHotkeys,
  downloadAndInstallUpdate,
  getAppVersion,
  getUpdateState,
  onUpdateState,
  onUpdaterProgress,
  openAccessibilitySettings,
  openExternalUrl,
  replayOnboarding,
  revealDevBinaryPath,
  uninstallApp,
  type DevBinaryInfo,
  type DiagnoseHotkeys,
  type UpdateState,
  type UpdaterProgress,
} from "@/lib/ipc";
import { Card } from "../components/Card";
import { Row } from "../components/Row";
import { SectionHeader } from "../components/SectionHeader";
import { formatBytes, formatDiagnostics } from "../helpers";
import { useConfirm } from "../useConfirm";

export function AboutTab() {
  const [version] = createResource(() => getAppVersion());
  const [checking, setChecking] = createSignal(false);
  // `update` is the authoritative state — sourced from the in-process
  // `UpdaterState` on the Rust side, hydrated on mount, and refreshed
  // by the `onUpdateState` subscription so a background check that
  // lands while Settings is open updates the row in real time.
  const [update, setUpdate] = createSignal<UpdateState | null>(null);
  const [installing, setInstalling] = createSignal(false);
  const [installError, setInstallError] = createSignal<string | null>(null);
  const [installProgress, setInstallProgress] =
    createSignal<UpdaterProgress | null>(null);
  // Toggle for the release notes section in the highlighted pending
  // state. Off by default so the row stays compact; "Release notes"
  // button expands it.
  const [showReleaseNotes, setShowReleaseNotes] = createSignal(false);
  const [diag, setDiag] = createSignal<DiagnoseHotkeys | null>(null);
  const [diagErr, setDiagErr] = createSignal<string | null>(null);
  const [devInfo, setDevInfo] = createSignal<DevBinaryInfo | null>(null);
  const [devCopied, setDevCopied] = createSignal(false);
  const [uninstalling, setUninstalling] = createSignal(false);
  // Two-step confirm. WKWebView swallows native `window.confirm()`, so the
  // destructive uninstall is gated behind a transient armed state that
  // auto-dismisses after 5 s.
  const confirmUninstall = useConfirm();

  // Load dev binary info eagerly so the path chip is visible without
  // requiring the user to click first.
  onMount(async () => {
    try {
      setDevInfo(await revealDevBinaryPath());
    } catch (err) {
      console.warn("revealDevBinaryPath failed:", err);
    }
  });

  // Subscribe to updater progress for the entire AboutTab lifetime.
  // Disposed by onCleanup. The Rust side emits `started` → `progress*` →
  // `installing` → `done`/`error`; on success the process restarts so
  // `done` may not be observed.
  let unlistenProgress: (() => void) | undefined;
  let unlistenUpdate: (() => void) | undefined;
  onMount(async () => {
    try {
      unlistenProgress = await onUpdaterProgress((p) => {
        setInstallProgress(p);
        if (p.phase === "error") {
          setInstallError(p.message);
          setInstalling(false);
        }
      });
    } catch (err) {
      console.warn("onUpdaterProgress failed:", err);
    }
    // Snapshot the in-process updater state, then subscribe to live
    // changes. The state is shared with the tray popover banner +
    // tray-icon dot; this is where Settings → About participates in
    // that single source of truth so a background check landing
    // while the window is open updates the row in real time.
    try {
      setUpdate(await getUpdateState());
    } catch (err) {
      console.warn("getUpdateState failed:", err);
    }
    try {
      unlistenUpdate = await onUpdateState((s) => {
        setUpdate(s);
        if (s.status !== "checking") setChecking(false);
      });
    } catch (err) {
      console.warn("onUpdateState failed:", err);
    }
  });
  onCleanup(() => {
    unlistenProgress?.();
    unlistenUpdate?.();
  });

  // Open System Settings only when the user actually needs it: a dev
  // build (sig rotates, Accessibility entry stale) or Accessibility
  // currently denied. Otherwise just copy the path.
  const shouldAutoOpenAxSettings = () => {
    const d = devInfo();
    if (!d) return false;
    return d.is_dev_build || !d.accessibility_granted;
  };

  async function onReplay() {
    try {
      await replayOnboarding();
    } catch (err) {
      console.error(err);
    }
  }

  async function onDiagnostics() {
    setDiagErr(null);
    setDiag(null);
    try {
      setDiag(await diagnoseHotkeys());
    } catch (err) {
      setDiagErr(String(err));
    }
  }

  async function onRevealDevPath() {
    try {
      const info = await revealDevBinaryPath();
      setDevInfo(info);
      // Pre-copy so the user can paste into Privacy & Security →
      // Accessibility's Cmd+Shift+G "Go to folder" prompt.
      try {
        await navigator.clipboard.writeText(info.binary_path);
        setDevCopied(true);
      } catch {
        setDevCopied(false);
      }
      if (info.is_dev_build || !info.accessibility_granted) {
        await openAccessibilitySettings();
      }
    } catch (err) {
      console.error(err);
    }
  }

  async function onCheckUpdates() {
    // Manual check bypasses the 6 h cooldown by design. The
    // checkForUpdatesNow IPC routes through `updater::perform_check`
    // which also emits `updates:state`, so we don't have to set the
    // local signal manually — the listener subscribed in onMount
    // does it. We still flip `checking` here for the immediate
    // button feedback (the listener clears it when the resulting
    // state arrives).
    setChecking(true);
    setInstallError(null);
    setInstallProgress(null);
    try {
      const r = await checkForUpdatesNow();
      setUpdate(r);
    } catch (err) {
      // Most failure paths surface via the emitted `error` status,
      // but a transport-level rejection can throw here. Reflect it
      // in the local state so the UI doesn't get stuck on
      // "Checking…". Preserve `available_version`, `release_notes`,
      // and `dismissed_version` from the previous successful check —
      // an offline manual click shouldn't make a previously-found
      // pending release disappear from the UI.
      const msg = String(err);
      setUpdate((prev) =>
        prev
          ? { ...prev, status: "error", last_error: msg }
          : {
              status: "error",
              current_version: version() ?? "?",
              last_error: msg,
            },
      );
    } finally {
      setChecking(false);
    }
  }

  async function onInstallUpdate() {
    if (installing()) return;
    setInstalling(true);
    setInstallError(null);
    setInstallProgress(null);
    try {
      // Resolves only on failure — success ends with app.restart() in
      // Rust, which tears down this webview.
      await downloadAndInstallUpdate();
    } catch (err) {
      setInstallError(String(err));
      setInstalling(false);
    }
  }

  function progressLabel(p: UpdaterProgress): string {
    if (p.phase === "started") return "Starting download…";
    if (p.phase === "progress") {
      const total = p.content_length ?? null;
      if (total && total > 0) {
        const pct = Math.min(100, Math.round((p.downloaded / total) * 100));
        return `Downloading… ${pct}% (${formatBytes(p.downloaded)} / ${formatBytes(total)})`;
      }
      return `Downloading… ${formatBytes(p.downloaded)}`;
    }
    if (p.phase === "installing") return "Installing…";
    if (p.phase === "done") return "Restarting Vox…";
    return "";
  }

  function progressPercent(p: UpdaterProgress): number | null {
    if (p.phase !== "progress") return null;
    const total = p.content_length ?? null;
    if (!total || total <= 0) return null;
    return Math.min(100, Math.round((p.downloaded / total) * 100));
  }

  /// Render a Unix-epoch seconds timestamp as a short relative
  /// English phrase ("just now", "5 min ago", "today", "yesterday",
  /// "3 days ago"). The "Released ___" body line uses this; the
  /// "Last checked ___" demotion-row uses it too.
  function relativeTime(secs: number): string {
    const nowMs = Date.now();
    const thenMs = secs * 1000;
    const diff = Math.max(0, nowMs - thenMs);
    const min = Math.round(diff / 60_000);
    if (min < 1) return "just now";
    if (min < 60) return `${min} min ago`;
    const hr = Math.round(min / 60);
    if (hr < 24) return `${hr} hr ago`;
    const day = Math.round(hr / 24);
    if (day === 1) return "yesterday";
    if (day < 7) return `${day} days ago`;
    if (day < 30) return `${Math.round(day / 7)} wk ago`;
    return `${Math.round(day / 30)} mo ago`;
  }

  async function onUninstall() {
    if (uninstalling()) return;
    setUninstalling(true);
    try {
      await uninstallApp();
      // The backend exits the process; we don't expect this to return.
    } catch (err) {
      console.error("uninstall failed:", err);
      setUninstalling(false);
      confirmUninstall.disarm();
    }
  }

  return (
    <>
      <div class="liquid-card mb-5 rounded-[14px] p-5 ring-1 ring-[var(--color-border)]">
        <div class="flex items-center gap-3">
          <VoxLogo size={48} />

          <div class="flex flex-col">
            <div class="text-[20px] font-semibold tracking-[-0.015em] text-[var(--color-ink)]">
              Vox
            </div>
            <div class="text-[12px] text-[var(--color-ink-mute)]">
              On-device dictation. Press, speak, paste.
            </div>
          </div>
        </div>
        <div class="mt-3 flex flex-wrap items-center gap-1.5">
          <span class="rounded-full bg-[var(--color-control)] px-2 py-0.5 text-[10px] uppercase tracking-[0.08em] text-[var(--color-ink-mute)]">
            v{version() ?? "—"}
          </span>
          <span class="rounded-full bg-[var(--color-success)]/15 px-2 py-0.5 text-[10px] uppercase tracking-[0.08em] text-[var(--color-success)]">
            Local-first
          </span>
        </div>
      </div>

      <SectionHeader title="Updates" />
      <Card>
        <Show
          when={update()?.status === "available" && update()?.available_version}
          fallback={
            // No pending update. Keep the legacy compact "Check now"
            // row + transient response state. The body text mentions
            // the last-checked timestamp so the user has *some*
            // signal that the background scheduler is doing its job.
            <Row
              label="Check for updates"
              hint={
                update()?.last_check_at
                  ? `Last checked ${relativeTime(update()!.last_check_at!)}. Vox checks once a day in the background.`
                  : "Vox checks once a day in the background. Click to check now."
              }
              last={!update()?.last_error && update()?.status !== "up-to-date"}
            >
              <button
                type="button"
                onClick={onCheckUpdates}
                disabled={checking() || installing()}
                class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)] disabled:opacity-40"
              >
                {checking() || update()?.status === "checking"
                  ? "Checking…"
                  : "Check now"}
              </button>
            </Row>
          }
        >
          {/* Highlighted pending state. The card itself stays
              `liquid-card`-styled (inherited from Card); the
              accent-soft background washes the row so it reads as
              "primary surface". Tokens reused: --color-accent,
              --color-accent-soft, --color-on-accent. */}
          <div class="flex flex-col gap-3 bg-[var(--color-accent-soft)] px-4 py-4">
            <div class="flex items-start gap-3">
              <span
                class="mt-0.5 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-[var(--color-accent)] text-[var(--color-on-accent)]"
                aria-hidden="true"
              >
                <SparkleGlyph width={16} height={16} />
              </span>
              <div class="flex min-w-0 flex-1 flex-col gap-1">
                <span class="text-[14px] font-semibold text-[var(--color-ink)]">
                  Vox v{update()!.available_version} is ready
                </span>
                <span class="text-[12px] leading-[1.5] text-[var(--color-ink-mute)]">
                  You're on v{update()!.current_version}.
                  <Show when={update()?.last_check_at}>
                    {" "}
                    Last checked {relativeTime(update()!.last_check_at!)}.
                  </Show>
                </span>
              </div>
            </div>
            <div class="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={onInstallUpdate}
                disabled={installing()}
                class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1.5 text-[12px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:cursor-wait disabled:opacity-70"
              >
                {installing() ? "Installing…" : "Install update"}
              </button>
              <Show when={update()?.release_notes}>
                <button
                  type="button"
                  onClick={() => setShowReleaseNotes(!showReleaseNotes())}
                  class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] text-[var(--color-ink)] hover:bg-[var(--color-control-hover)]"
                  aria-expanded={showReleaseNotes()}
                >
                  Release notes
                </button>
              </Show>
              <button
                type="button"
                onClick={onCheckUpdates}
                disabled={checking() || installing()}
                class="rounded-[6px] px-2 py-1.5 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)] disabled:opacity-40"
                title="Re-check for updates"
              >
                {checking() || update()?.status === "checking"
                  ? "Checking…"
                  : "Check again"}
              </button>
            </div>
            <Show when={showReleaseNotes() && update()?.release_notes}>
              <div class="rounded-[8px] bg-[var(--color-control)]/60 px-3 py-2.5 ring-1 ring-[var(--color-border)]">
                <div class="mb-1 text-[10px] font-medium uppercase tracking-[0.08em] text-[var(--color-ink-mute)]">
                  What's new
                </div>
                <pre class="m-0 max-h-40 overflow-auto whitespace-pre-wrap font-sans text-[12px] leading-[1.5] text-[var(--color-ink)]">{update()!.release_notes}</pre>
              </div>
            </Show>
            <Show when={installing() && installProgress()}>
              {(p) => (
                <div class="flex flex-col gap-1.5">
                  <div class="text-[11px] text-[var(--color-ink-mute)]">
                    {progressLabel(p())}
                  </div>
                  <div class="h-1.5 w-full overflow-hidden rounded-full bg-[var(--color-control)]">
                    <Show
                      when={progressPercent(p()) !== null}
                      fallback={
                        <div class="h-full w-1/3 animate-pulse rounded-full bg-[var(--color-accent)]" />
                      }
                    >
                      <div
                        class="h-full rounded-full bg-[var(--color-accent)] transition-[width] duration-200"
                        style={{ width: `${progressPercent(p())}%` }}
                      />
                    </Show>
                  </div>
                </div>
              )}
            </Show>
            <Show when={installing() && !installProgress()}>
              <div class="text-[11px] text-[var(--color-ink-mute)]">
                Downloading… Vox will relaunch.
              </div>
            </Show>
            <Show when={installError()}>
              <div class="text-[11px] text-[var(--color-danger)]">
                {installError()}
              </div>
            </Show>
          </div>
        </Show>
        <Show when={update()?.status === "up-to-date"}>
          <div class="border-t border-[var(--color-border)] px-4 py-3 text-[12px] text-[var(--color-ink-mute)]">
            Vox is up to date.
          </div>
        </Show>
        <Show when={update()?.status === "unconfigured"}>
          <div class="border-t border-[var(--color-border)] px-4 py-3 text-[12px] text-[var(--color-ink-mute)]">
            {update()?.last_error ?? "Auto-update isn't configured for this build."}
          </div>
        </Show>
        <Show when={update()?.status === "error" && update()?.last_error}>
          <div class="border-t border-[var(--color-border)] px-4 py-3 text-[12px] text-[var(--color-danger)]">
            {update()!.last_error}
          </div>
        </Show>
      </Card>

      <SectionHeader title="Help & diagnostics" />
      <Card>
        <Row
          label="Report a bug"
          hint="Found something broken? Open an issue on GitHub."
        >
          <button
            type="button"
            onClick={() =>
              void openExternalUrl("https://github.com/KlymSerhii/vox-releases/issues")
            }
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            Open issues
          </button>
        </Row>
        <Row
          label="Replay onboarding"
          hint="Walk through the setup wizard again."
        >
          <button
            type="button"
            onClick={onReplay}
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            Start
          </button>
        </Row>
        <Row
          label="Hotkey diagnostics"
          hint="Print bindings and any registration errors."
        >
          <button
            type="button"
            onClick={() => void onDiagnostics()}
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            Run
          </button>
        </Row>
        <Row
          label="Binary path for Accessibility"
          hint="Dev-build only. macOS keys Accessibility grants by signature, which rotates each rebuild; copy this path to re-add Vox under Privacy & Security if needed."
          last={!devInfo() && !diagErr() && !diag()}
        >
          <button
            type="button"
            onClick={() => void onRevealDevPath()}
            class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
          >
            {shouldAutoOpenAxSettings() ? "Open & copy path" : "Copy path"}
          </button>
        </Row>
        <Show when={devInfo()}>
          {(d) => (
            <div class="flex flex-col gap-1 border-t border-[var(--color-border)] px-4 py-3">
              <span class="text-[11px] text-[var(--color-ink-mute)]">
                Bundle id:{" "}
                <code class="text-[var(--color-ink)]">
                  {d().bundle_identifier}
                </code>
                <Show when={d().is_dev_build}>
                  <span class="ml-2 rounded-full bg-[var(--color-accent)]/20 px-2 py-0.5 text-[10px] text-[var(--color-accent)]">
                    dev build
                  </span>
                </Show>
                <Show when={!d().accessibility_granted}>
                  <span class="ml-2 rounded-full bg-[var(--color-danger)]/20 px-2 py-0.5 text-[10px] text-[var(--color-danger)]">
                    accessibility denied
                  </span>
                </Show>
              </span>
              <span class="break-all text-[11px] text-[var(--color-ink-faint)]">
                {d().binary_path}
              </span>
              <Show when={devCopied()}>
                <span class="text-[11px] text-[var(--color-success)]">
                  Path copied to clipboard.
                </span>
              </Show>
            </div>
          )}
        </Show>
        <Show when={diagErr()}>
          <div class="border-t border-[var(--color-border)] px-4 py-3 text-[12px] text-[var(--color-danger)]">
            {diagErr()}
          </div>
        </Show>
        <Show when={diag()}>
          {(d) => (
            <div class="border-t border-[var(--color-border)] px-4 py-3">
              <pre class="overflow-x-auto rounded-[6px] bg-[var(--color-input-bg)] p-3 text-[11px] text-[var(--color-ink)]">
                {formatDiagnostics(d())}
              </pre>
            </div>
          )}
        </Show>
      </Card>

      <SectionHeader title="Open-source attributions" />
      <Card>
        <div class="flex flex-col gap-3 px-4 py-3 text-[12px] leading-[1.55] text-[var(--color-ink-mute)]">
          <p>
            Speech recognition powered by{" "}
            <a
              href="https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3"
              onClick={(e) => {
                e.preventDefault();
                void openExternalUrl(
                  "https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3",
                );
              }}
              class="text-[var(--color-accent)] hover:underline"
            >
              NVIDIA Parakeet-TDT-0.6B-v3
            </a>{" "}
            (CC-BY-4.0). CoreML conversion by{" "}
            <a
              href="https://github.com/FluidInference/FluidAudio"
              onClick={(e) => {
                e.preventDefault();
                void openExternalUrl(
                  "https://github.com/FluidInference/FluidAudio",
                );
              }}
              class="text-[var(--color-accent)] hover:underline"
            >
              FluidInference
            </a>{" "}
            (Apache-2.0).
          </p>
          <p>
            Whisper by{" "}
            <a
              href="https://github.com/openai/whisper"
              onClick={(e) => {
                e.preventDefault();
                void openExternalUrl("https://github.com/openai/whisper");
              }}
              class="text-[var(--color-accent)] hover:underline"
            >
              OpenAI
            </a>{" "}
            (MIT). Inference via{" "}
            <a
              href="https://github.com/ggerganov/whisper.cpp"
              onClick={(e) => {
                e.preventDefault();
                void openExternalUrl(
                  "https://github.com/ggerganov/whisper.cpp",
                );
              }}
              class="text-[var(--color-accent)] hover:underline"
            >
              whisper.cpp
            </a>{" "}
            (MIT).
          </p>
          <p>
            Polishing engines: Apple Foundation Models (system) and{" "}
            <a
              href="https://ai.google.dev/gemma"
              onClick={(e) => {
                e.preventDefault();
                void openExternalUrl("https://ai.google.dev/gemma");
              }}
              class="text-[var(--color-accent)] hover:underline"
            >
              Gemma
            </a>{" "}
            by Google (Gemma terms).
          </p>
        </div>
      </Card>

      <SectionHeader title="Danger zone" />
      <Card>
        <Row
          label="Uninstall Vox"
          hint="Wipes settings, voice modes, downloaded models, and the local llama-server. Resets Microphone + Accessibility permissions, disables Launch at Login, then quits. To finish uninstalling, drag Vox.app from /Applications to the Trash."
          last
        >
          <Show when={!confirmUninstall.armed()}>
            <button
              type="button"
              onClick={() => confirmUninstall.armOrFire(() => void onUninstall())}
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] text-[var(--color-ink-mute)] hover:bg-[var(--color-danger)]/20 hover:text-[var(--color-danger)]"
            >
              Uninstall…
            </button>
          </Show>
          <Show when={confirmUninstall.armed()}>
            <button
              type="button"
              onClick={confirmUninstall.disarm}
              disabled={uninstalling()}
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => confirmUninstall.armOrFire(() => void onUninstall())}
              disabled={uninstalling()}
              class="rounded-[6px] bg-[var(--color-danger)] px-3 py-1.5 text-[12px] font-medium text-white hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {uninstalling() ? "Uninstalling…" : "Confirm uninstall"}
            </button>
          </Show>
        </Row>
      </Card>
    </>
  );
}
