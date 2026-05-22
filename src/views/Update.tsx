import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { SparkleGlyph } from "@/components/SparkleGlyph";
import {
  downloadAndInstallUpdate,
  getUpdateState,
  onUpdaterProgress,
  onUpdateState,
  type UpdateState,
} from "@/lib/ipc";

/**
 * Dedicated "a new version is available" window. Auto-appears once per
 * available version when a background update check finds a release; the
 * Rust side records the version as shown before this renders, so the
 * window fires at most once per version, ever.
 *
 * Mirrors the tray popover's `UpdateBanner` — same accent-soft card,
 * `SparkleGlyph`, and `--color-*` tokens (no new gradients/shadows) —
 * scaled up to fill the pop-up window, with the release notes added in
 * a scrollable region. Two actions:
 *
 *   - Install now → download + install (the app restarts on success);
 *                   the button disables while installing.
 *   - Later       → just close the window. The passive tray banner +
 *                   dot remain, so we deliberately do NOT dismiss.
 *
 * Coexists with the banner — closing here records nothing extra and
 * leaves the banner showing for as long as the update is pending and
 * undismissed.
 */
export default function UpdateView() {
  const [state, setState] = createSignal<UpdateState | null>(null);
  // Doubles as the double-fire guard on Install — a second click while
  // the download await is in flight is a no-op.
  const [installing, setInstalling] = createSignal(false);

  let unlistenState: (() => void) | undefined;
  let unlistenProgress: (() => void) | undefined;

  onMount(async () => {
    try {
      setState(await getUpdateState());
    } catch (err) {
      console.warn("update: getUpdateState failed", err);
    }
    // Live-update if the state changes while the window is open (e.g. a
    // follow-up check moves to up-to-date).
    unlistenState = await onUpdateState(setState);
    // Drive the Installing… state the same way the tray banner does —
    // the app restarts on success, so we never read a finished bar.
    unlistenProgress = await onUpdaterProgress((p) => {
      if (p.phase === "error" || p.phase === "done") {
        setInstalling(false);
      } else {
        setInstalling(true);
      }
    });
  });

  onCleanup(() => {
    unlistenState?.();
    unlistenProgress?.();
  });

  function close() {
    void getCurrentWindow().close();
  }

  async function install() {
    if (installing()) return;
    setInstalling(true);
    try {
      // Resolves only on failure — success ends with `app.restart()` on
      // the Rust side, which tears this window down. The progress
      // listener drives the disabled state in either direction.
      await downloadAndInstallUpdate();
    } catch (err) {
      console.error("update: install failed", err);
      setInstalling(false);
    }
  }

  const availableVersion = () => state()?.available_version ?? "";
  const currentVersion = () => state()?.current_version ?? "";
  const releaseNotes = () => (state()?.release_notes ?? "").trim();

  return (
    <div class="flex h-screen w-screen flex-col bg-[var(--color-bg)] px-8 py-9 text-[var(--color-ink)]">
      <div class="flex flex-col items-center gap-5 text-center">
        <span
          class="inline-flex h-14 w-14 shrink-0 items-center justify-center rounded-full bg-[var(--color-accent-soft)] text-[var(--color-accent)] ring-1 ring-[var(--color-border)]"
          aria-hidden="true"
        >
          <SparkleGlyph width={26} height={26} />
        </span>

        <div class="flex flex-col gap-2">
          <h1 class="text-[19px] font-semibold leading-tight">
            A new version of Vox is available
          </h1>
          <p class="text-[14px] font-medium text-[var(--color-accent)]">
            v{availableVersion()}
          </p>
          <Show when={currentVersion()}>
            <p class="text-[12px] text-[var(--color-ink-mute)]">
              You're on v{currentVersion()}
            </p>
          </Show>
        </div>
      </div>

      <Show when={releaseNotes()}>
        <div class="mt-6 flex min-h-0 flex-1 flex-col">
          <span class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-[var(--color-ink-mute)]">
            What's new
          </span>
          <div class="min-h-0 flex-1 overflow-y-auto rounded-[10px] bg-[var(--color-accent-soft)] px-4 py-3 text-left text-[13px] leading-relaxed whitespace-pre-wrap ring-1 ring-[var(--color-border)]">
            {releaseNotes()}
          </div>
        </div>
      </Show>

      <div
        class="flex w-full flex-col items-stretch gap-2.5"
        classList={{ "mt-6": !releaseNotes(), "mt-5": !!releaseNotes() }}
      >
        <button
          type="button"
          class="inline-flex h-10 items-center justify-center rounded-[8px] bg-[var(--color-accent)] px-4 text-[13px] font-medium text-[var(--color-on-accent)] transition-[filter] hover:brightness-110 disabled:cursor-wait disabled:opacity-70"
          disabled={installing()}
          onClick={() => void install()}
        >
          <Show when={!installing()} fallback={<span>Installing…</span>}>
            <span>Install now</span>
          </Show>
        </button>
        <button
          type="button"
          class="rounded-[8px] px-4 py-2 text-[12px] text-[var(--color-ink-mute)] transition-colors hover:text-[var(--color-ink)] disabled:cursor-not-allowed disabled:opacity-40"
          disabled={installing()}
          onClick={close}
        >
          Later
        </button>
      </div>
    </div>
  );
}
