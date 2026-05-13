import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  cancelSession,
  checkPermissions,
  closeTrayPopover,
  copyText,
  getPublicSettings,
  getTimeSavedStats,
  inferenceServerStatus,
  listAudioInputDevices,
  listModels,
  listModes,
  onSessionsChanged,
  onSettingsChanged,
  onStatsChanged,
  openAccessibilitySettings,
  openSettingsWindow,
  quitApp,
  rePolishTranscript,
  sessionsSnapshot,
  setActiveMode,
  setAppearance,
  setAudioInputDevice,
  setAutoPasteEnabled,
  setEnableThinking,
  setHandsFreeEnabled,
  setPolishingEnabled,
  setStreamingPillEnabled,
  type AudioInputDevice,
  type CompletedEntry,
  type InProgressEntry,
  type ModelStatus,
  type Period,
  type SessionsSnapshot,
  type Settings,
  type Stage,
  type TimeSavedStats,
  type VoiceMode,
} from "../lib/ipc";

export default function TrayPopoverView() {
  const [loaded, setLoaded] = createSignal(false);
  const [s, setS] = createSignal<Settings | null>(null);
  const [recents, setRecents] = createSignal<CompletedEntry[]>([]);
  const [inProgress, setInProgress] = createSignal<InProgressEntry[]>([]);
  const [copyFlash, setCopyFlash] = createSignal<number | null>(null);
  // Per-row "Re-polished as X" hint timer keyed by session id. Cleared
  // after a short delay so the row collapses back to the default look.
  const [repolishFlash, setRepolishFlash] = createSignal<{
    sessionId: number;
    modeName: string;
    warning: boolean;
  } | null>(null);
  // Sessions currently being re-polished (background LLM call in
  // flight). Used to render a small spinner / disable the picker on
  // the affected row.
  const [repolishBusy, setRepolishBusy] = createSignal<Set<number>>(new Set());
  // Per-row picker state. Only one row's picker is open at a time so
  // the dense popover doesn't pile up flyouts. Stored as the row's
  // session id (or null for closed).
  const [repolishPickerSessionId, setRepolishPickerSessionId] = createSignal<
    number | null
  >(null);
  let repolishPickerRef: HTMLDivElement | undefined;
  const repolishTriggerRefs = new Map<number, HTMLButtonElement>();
  const [repolishMenuPos, setRepolishMenuPos] = createSignal<{
    placement: "below" | "above";
    top: number;
    bottom: number;
    left: number;
    minWidth: number;
    maxHeight: number;
  }>({
    placement: "below",
    top: 0,
    bottom: 0,
    left: 0,
    minWidth: 0,
    maxHeight: 240,
  });
  // Per-row inline error (cleared on retry / outside-click). Keyed by
  // session id so error from one row doesn't shadow another.
  const [repolishError, setRepolishError] = createSignal<{
    sessionId: number;
    message: string;
  } | null>(null);
  const [modes, setModes] = createSignal<VoiceMode[]>([]);
  const [modeMenuOpen, setModeMenuOpen] = createSignal(false);
  // Header status uses these to detect "models not yet installed". A
  // non-empty configured id whose ModelStatus reports `installed: false`
  // means the user is past onboarding but the artifact didn't survive
  // (manual delete, partial download). The header should say
  // "Setup needed" so they don't trust the "ready" badge for a pipeline
  // that will fail on first use.
  const [models, setModels] = createSignal<ModelStatus[]>([]);
  const [inferenceInstalled, setInferenceInstalled] = createSignal<boolean | null>(null);
  const [period, setPeriod] = createSignal<Period>("day");
  const [stats, setStats] = createSignal<TimeSavedStats>({
    saved_seconds: 0,
    sessions: 0,
  });
  const [quickSettingsOpen, setQuickSettingsOpen] = createSignal(false);
  // Accessibility grant status for the auto-paste toggle. `null` =
  // not yet probed; treated as "ungranted" for nudging purposes until
  // the first probe lands. Re-probed on mount and whenever the
  // popover regains focus (the user may have flipped the grant in
  // System Settings while the popover was hidden).
  const [accessibility, setAccessibility] = createSignal<boolean | null>(null);
  // Anchor for the mode-picker dropdown. Used by Show-when-open to
  // position the listbox via fixed coordinates so it isn't clipped by
  // the popover root's `overflow-hidden`.
  let modeTriggerRef: HTMLButtonElement | undefined;
  let modeMenuRef: HTMLDivElement | undefined;
  const [modeMenuPos, setModeMenuPos] = createSignal<{
    top: number;
    left: number;
    minWidth: number;
  }>({ top: 0, left: 0, minWidth: 0 });
  const [periodMenuOpen, setPeriodMenuOpen] = createSignal(false);
  let periodTriggerRef: HTMLButtonElement | undefined;
  let periodMenuRef: HTMLDivElement | undefined;
  const [periodMenuPos, setPeriodMenuPos] = createSignal<{
    top: number;
    left: number;
    minWidth: number;
  }>({ top: 0, left: 0, minWidth: 0 });
  // Refreshed on popover mount + every time the window regains focus
  // so plugging an AirPods doesn't leave the dropdown stale.
  const [audioDevices, setAudioDevices] = createSignal<AudioInputDevice[]>([]);
  const [micMenuOpen, setMicMenuOpen] = createSignal(false);
  let micTriggerRef: HTMLButtonElement | undefined;
  let micMenuRef: HTMLDivElement | undefined;
  // The Mic row sits in the middle of the popover, so a long device
  // list spills past the bottom edge. Mirror the repolish picker's
  // flip-up logic — when there isn't room below, anchor by `bottom`
  // and pin `max-height` to whichever side we landed on so the menu
  // scrolls instead of clipping at the OS window edge.
  const [micMenuPos, setMicMenuPos] = createSignal<{
    placement: "below" | "above";
    top: number;
    bottom: number;
    left: number;
    minWidth: number;
    maxHeight: number;
  }>({
    placement: "below",
    top: 0,
    bottom: 0,
    left: 0,
    minWidth: 0,
    maxHeight: 240,
  });
  const PERIOD_OPTIONS: { value: Period; label: string }[] = [
    { value: "last_session", label: "Last session" },
    { value: "day", label: "Today" },
    { value: "week", label: "Week" },
    { value: "month", label: "Month" },
    { value: "year", label: "Year" },
  ];
  const periodLabel = () =>
    PERIOD_OPTIONS.find((o) => o.value === period())?.label ?? "Today";

  let unlistenSettings: (() => void) | null = null;
  let unlistenSessions: (() => void) | null = null;
  let unlistenStats: (() => void) | null = null;

  async function refreshStats() {
    try {
      setStats(await getTimeSavedStats(period()));
    } catch (e) {
      console.warn(e);
      setStats({ saved_seconds: 0, sessions: 0 });
    }
  }

  function applySnapshot(snap: SessionsSnapshot) {
    setInProgress(snap.in_progress);
    // RECENT keeps the richer `CompletedEntry` shape so the per-row
    // re-polish action has the `session_id` it needs to call back
    // into the backend. The old `transcripts:changed` projection is
    // still emitted by the Rust side, but we drop the legacy
    // subscription here — `sessions:changed` carries the same data
    // (and more), so there's no value in maintaining both wiring.
    setRecents(snap.recent);
  }

  onMount(async () => {
    document.body.classList.add("translucent");
    try {
      const [settings, snap, list] = await Promise.all([
        getPublicSettings(),
        sessionsSnapshot(),
        listModes(),
      ]);
      setS(settings);
      applySnapshot(snap);
      setModes(list);
      // Probe installed-model status so the header can flag
      // "Setup needed" when the configured ids aren't on disk yet.
      // Failures here are non-fatal — we fall back to the conservative
      // "is the configured id non-empty" check.
      try {
        const [allModels, infer] = await Promise.all([
          listModels(),
          inferenceServerStatus(),
        ]);
        setModels(allModels);
        setInferenceInstalled(infer.installed);
      } catch (err) {
        console.warn("model status probes failed:", err);
      }
      try {
        setAudioDevices(await listAudioInputDevices());
      } catch (err) {
        console.warn("listAudioInputDevices failed:", err);
      }
      // Probe Accessibility so the auto-paste nudge can render
      // correctly on the very first popover open. Non-fatal.
      void refreshAccessibility();
    } finally {
      setLoaded(true);
    }

    // The popover webview is built once and reused on every tray click,
    // so onMount only runs the first time. Live events keep state fresh
    // across reopens; the focus handler is a backstop for events that
    // fire while the popover is hidden.
    unlistenSettings = await onSettingsChanged(setS);
    unlistenSessions = await onSessionsChanged(applySnapshot);
    unlistenStats = await onStatsChanged(refreshStats);
    void refreshStats();

    window.addEventListener("focus", () => {
      void sessionsSnapshot().then(applySnapshot);
      void listAudioInputDevices()
        .then(setAudioDevices)
        .catch((err) => console.warn("listAudioInputDevices failed:", err));
      // User may have flipped Accessibility in System Settings while
      // the popover was hidden — keep the nudge in sync.
      void refreshAccessibility();
    });

    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape") closeTrayPopover();
    });

    // Outside-click handler closes the fixed-positioned mode-picker
    // dropdown. Without it, the menu can persist after losing focus
    // and reappear out-of-context when the popover hides + re-shows.
    document.addEventListener("mousedown", (e) => {
      const target = e.target as Node | null;
      if (!target) return;
      if (modeMenuOpen()) {
        const inMenu = !!modeMenuRef && modeMenuRef.contains(target);
        const inTrigger =
          !!modeTriggerRef && modeTriggerRef.contains(target);
        if (!inMenu && !inTrigger) setModeMenuOpen(false);
      }
      if (periodMenuOpen()) {
        const inMenu = !!periodMenuRef && periodMenuRef.contains(target);
        const inTrigger =
          !!periodTriggerRef && periodTriggerRef.contains(target);
        if (!inMenu && !inTrigger) setPeriodMenuOpen(false);
      }
      if (micMenuOpen()) {
        const inMenu = !!micMenuRef && micMenuRef.contains(target);
        const inTrigger = !!micTriggerRef && micTriggerRef.contains(target);
        if (!inMenu && !inTrigger) setMicMenuOpen(false);
      }
      const openId = repolishPickerSessionId();
      if (openId !== null) {
        const inMenu =
          !!repolishPickerRef && repolishPickerRef.contains(target);
        const trigger = repolishTriggerRefs.get(openId);
        const inTrigger = !!trigger && trigger.contains(target);
        if (!inMenu && !inTrigger) setRepolishPickerSessionId(null);
      }
    });

    // Tauri fires this when the popover loses key window status, e.g.
    // when the user clicks outside the popover entirely.
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().onFocusChanged(({ payload }) => {
        if (!payload) {
          setModeMenuOpen(false);
          setPeriodMenuOpen(false);
          setRepolishPickerSessionId(null);
          setMicMenuOpen(false);
        }
      });
    } catch (err) {
      console.warn("focus listener init failed:", err);
    }
  });

  onCleanup(() => {
    unlistenSettings?.();
    unlistenSessions?.();
    unlistenStats?.();
  });

  async function pickMode(id: string) {
    const cur = s();
    if (!cur) return;
    setModeMenuOpen(false);
    try {
      await setActiveMode(id);
      setS({ ...cur, active_mode_id: id });
    } catch (err) {
      console.error("setActiveMode failed:", err);
    }
  }

  // `set_settings` is gated to the settings/onboarding windows, so
  // the tray uses the dedicated `set_audio_input_device` IPC — same
  // path the Settings UI now goes through. The backend probes the
  // device on a background thread to surface the macOS Continuity
  // Microphone wake UI when the user picks an iPhone.
  async function pickMic(name: string | null) {
    const cur = s();
    if (!cur) return;
    setMicMenuOpen(false);
    if ((cur.audio_input_device ?? null) === name) return;
    const next = { ...cur, audio_input_device: name } as Settings;
    setS(next);
    try {
      await setAudioInputDevice(name);
    } catch (err) {
      console.error("setAudioInputDevice failed:", err);
      // Roll back the optimistic update so the dropdown reflects
      // what's actually on disk.
      setS(cur);
    }
  }

  function micLabel(): string {
    const v = s()?.audio_input_device;
    if (!v) return "System default";
    return v;
  }

  function micIsCustom(): boolean {
    return !!s()?.audio_input_device;
  }

  function activeMode(): VoiceMode | undefined {
    const id = s()?.active_mode_id;
    if (!id) return undefined;
    return modes().find((m) => m.id === id) ?? modes()[0];
  }

  async function toggleHandsFree(v: boolean) {
    const cur = s();
    if (!cur) return;
    setS({ ...cur, hands_free_enabled: v });
    await setHandsFreeEnabled(v);
  }

  async function togglePolishing(v: boolean) {
    const cur = s();
    if (!cur) return;
    // Optimistically flip only the toggle's own field. The backend's
    // `set_polishing_enabled` preserves the user's chosen `llm_engine`
    // (e.g. apple-fm) — hardcoding it to "gemma-local"/"none" here
    // would clobber that choice for the brief window before
    // `settings:changed` reconciles. Engine name reflows naturally
    // through the onSettingsChanged listener.
    setS({ ...cur, polishing_enabled: v });
    await setPolishingEnabled(v);
  }

  async function toggleStreamingPill(v: boolean) {
    const cur = s();
    if (!cur) return;
    setS({ ...cur, streaming_pill_enabled: v });
    await setStreamingPillEnabled(v);
  }

  async function refreshAccessibility() {
    try {
      const r = await checkPermissions();
      setAccessibility(r.accessibility);
    } catch (e) {
      console.warn("checkPermissions failed:", e);
      setAccessibility(false);
    }
  }

  async function toggleAutoPaste(v: boolean) {
    const cur = s();
    if (!cur) return;
    setS({ ...cur, auto_paste_enabled: v });
    await setAutoPasteEnabled(v);
    // Re-probe on flip-on so the nudge surfaces immediately if
    // Accessibility isn't granted yet.
    if (v) await refreshAccessibility();
  }

  async function toggleEnableThinking(v: boolean) {
    const cur = s();
    if (!cur) return;
    setS({ ...cur, enable_thinking: v });
    await setEnableThinking(v);
  }

  async function copyRecent(idx: number, text: string) {
    try {
      await copyText(text);
      setCopyFlash(idx);
      setTimeout(() => setCopyFlash(null), 900);
    } catch (e) {
      console.error("copy failed:", e);
    }
  }

  function openRepolishPicker(sessionId: number) {
    const trigger = repolishTriggerRefs.get(sessionId);
    if (trigger) {
      const r = trigger.getBoundingClientRect();
      // Right-align to the trigger via fixed coords so the dropdown
      // can escape the popover root's overflow-hidden clipping. The
      // tray popover is a small window — a Recent row near the bottom
      // doesn't have room below for 6 modes, so flip the dropdown
      // above when there's more headroom there. Either way, cap
      // maxHeight to whichever side we land on so the list scrolls
      // instead of clipping past the window edge.
      const viewportH = window.innerHeight;
      const menuW = Math.max(r.width, 200);
      const left = Math.max(8, r.right - menuW);
      const spaceBelow = viewportH - r.bottom - 8;
      const spaceAbove = r.top - 8;
      // ~28px header + 6 modes × ~32px row; below this threshold the
      // flip-up branch is preferable if we have more room up there.
      const desiredH = 220;
      const placeBelow = spaceBelow >= desiredH || spaceBelow >= spaceAbove;
      if (placeBelow) {
        setRepolishMenuPos({
          placement: "below",
          top: r.bottom + 4,
          bottom: 0,
          left,
          minWidth: menuW,
          maxHeight: spaceBelow,
        });
      } else {
        setRepolishMenuPos({
          placement: "above",
          top: 0,
          bottom: viewportH - r.top + 4,
          left,
          minWidth: menuW,
          maxHeight: spaceAbove,
        });
      }
    }
    // Clear any stale per-row error so the picker opens clean.
    setRepolishError(null);
    setRepolishPickerSessionId(sessionId);
  }

  async function runRepolish(sessionId: number, modeId: string) {
    setRepolishPickerSessionId(null);
    setRepolishError(null);
    // Mark this session as busy so the row shows a spinner and the
    // trigger is disabled (immutable Set replace so Solid re-runs).
    setRepolishBusy((prev) => {
      const next = new Set(prev);
      next.add(sessionId);
      return next;
    });
    try {
      const result = await rePolishTranscript(sessionId, modeId);
      setRecents((rs) =>
        rs.map((r) =>
          r.session_id === sessionId
            ? {
                ...r,
                text: result.text,
                mode_id: result.mode_id,
                warning: result.warning,
              }
            : r,
        ),
      );
      setRepolishFlash({
        sessionId,
        modeName: result.mode_name,
        warning: result.warning,
      });
      // Match the existing copy-flash timing (~900ms) so success
      // confirmations feel consistent. Re-polish takes longer to
      // produce so the user actually sees the affirmation; we hold
      // for a beat longer than copy.
      setTimeout(() => {
        setRepolishFlash((f) => (f && f.sessionId === sessionId ? null : f));
      }, 2200);
    } catch (e) {
      const message = typeof e === "string" ? e : String(e);
      console.error("rePolishTranscript failed:", message);
      setRepolishError({ sessionId, message: shortRepolishError(message) });
      setTimeout(() => {
        setRepolishError((cur) =>
          cur && cur.sessionId === sessionId ? null : cur,
        );
      }, 3200);
    } finally {
      setRepolishBusy((prev) => {
        const next = new Set(prev);
        next.delete(sessionId);
        return next;
      });
    }
  }

  function statusLine(): { label: string; tone: "ok" | "off" | "warn" } {
    const cur = s();
    if (!cur) return { label: "Loading…", tone: "ok" };
    if (!cur.dictation_enabled) return { label: "Dictation off", tone: "off" };
    if (setupNeeded()) return { label: "Setup needed", tone: "warn" };
    return { label: "Local · ready", tone: "ok" };
  }

  /// True when the user has the polishing pipeline configured but a
  /// required artifact is missing. The header "Local · ready" badge
  /// would otherwise lie before the user has actually downloaded
  /// anything. Conservative fallback: when we couldn't probe (null
  /// `inferenceInstalled`), only flag missing configuration ids — once
  /// the probe lands the full check kicks in.
  function setupNeeded(): boolean {
    const cur = s();
    if (!cur) return false;
    if (!cur.stt_model_id) return true;
    const sttId = cur.stt_model_id;
    const llmId = cur.llm_model_id;
    const probed = models();
    if (probed.length > 0) {
      const sttOk = probed.find((m) => m.descriptor.id === sttId)?.installed ?? false;
      if (!sttOk) return true;
      // Apple FM has no GGUF/llama-server requirement.
      if (cur.llm_engine === "gemma-local") {
        if (!llmId) return true;
        const llmOk = probed.find((m) => m.descriptor.id === llmId)?.installed ?? false;
        if (!llmOk) return true;
        const inf = inferenceInstalled();
        if (inf === false) return true;
      }
    }
    return false;
  }

  return (
    <div class="liquid-glass-window flex h-screen w-screen flex-col overflow-hidden rounded-[14px] text-[13px] text-[var(--color-ink)]">
      <Show
        when={loaded() && s()}
        fallback={
          <div class="flex h-full items-center justify-center text-[var(--color-ink-mute)]">
            Loading…
          </div>
        }
      >
        {(st) => (
          <>
            <header class="flex shrink-0 items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
              <Show
                when={setupNeeded()}
                fallback={
                  <div class="flex items-center gap-2">
                    <StatusDot tone={statusLine().tone} />
                    <span class="text-[13px] font-semibold tracking-[-0.005em] text-[var(--color-ink)]">
                      {statusLine().label}
                    </span>
                  </div>
                }
              >
                <button
                  type="button"
                  class="-mx-1 inline-flex items-center gap-2 rounded-[6px] px-1 py-0.5 text-left transition-colors hover:bg-[var(--color-overlay)]"
                  onClick={() => openSettingsWindow()}
                  title="Open Settings → Models"
                >
                  <StatusDot tone={statusLine().tone} />
                  <span class="text-[13px] font-semibold tracking-[-0.005em] text-[var(--color-ink)]">
                    {statusLine().label}
                  </span>
                  <span class="text-[11px] text-[var(--color-ink-mute)]">
                    Open Models →
                  </span>
                </button>
              </Show>
              <button
                type="button"
                class="inline-flex h-7 w-7 items-center justify-center rounded-[6px] p-1.5 text-[var(--color-ink-mute)] transition-colors hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]"
                title="Open Settings"
                onClick={() => openSettingsWindow()}
              >
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.75"
                >
                  <circle cx="12" cy="12" r="3" />
                  <path d="M19.4 15a7.97 7.97 0 0 0 .1-3l2-1.5-2-3.4-2.3.8a8 8 0 0 0-2.6-1.5L14 3h-4l-.6 2.4a8 8 0 0 0-2.6 1.5l-2.3-.8-2 3.4L4.5 12a8 8 0 0 0 .1 3l-2 1.5 2 3.4 2.3-.8a8 8 0 0 0 2.6 1.5l.6 2.4h4l.6-2.4a8 8 0 0 0 2.6-1.5l2.3.8 2-3.4z" />
                </svg>
              </button>
            </header>

            <div class="flex min-h-0 flex-1 flex-col overflow-y-auto">
            <div class="border-b border-[var(--color-border)] px-4 py-3">
              <div class="liquid-card flex items-center gap-3 rounded-[10px] px-3 py-2.5 ring-1 ring-[var(--color-border)]">
                <div class="flex min-w-0 flex-1 flex-col">
                  <span class="text-[10px] font-semibold uppercase tracking-[0.06em] text-[var(--color-ink-mute)]">
                    Time saved
                  </span>
                  <span class="mt-0.5 text-[20px] font-semibold leading-none tracking-[-0.01em] text-[var(--color-ink)] tabular-nums">
                    {formatDuration(stats().saved_seconds)}
                  </span>
                  <span class="mt-1 text-[11px] leading-snug text-[var(--color-ink-mute)]">
                    {sessionsSubLine(stats().sessions)}
                  </span>
                </div>
                <button
                  ref={(el) => (periodTriggerRef = el)}
                  type="button"
                  class="liquid-card inline-flex shrink-0 items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium text-[var(--color-ink)] ring-1 ring-[var(--color-border-strong)] transition-[filter] hover:brightness-105"
                  onClick={() => {
                    const next = !periodMenuOpen();
                    if (next && periodTriggerRef) {
                      const r = periodTriggerRef.getBoundingClientRect();
                      const menuW = Math.max(r.width, 140);
                      const right = r.right;
                      const left = Math.max(8, right - menuW);
                      setPeriodMenuPos({
                        top: r.bottom + 4,
                        left,
                        minWidth: menuW,
                      });
                    }
                    setPeriodMenuOpen(next);
                  }}
                  aria-haspopup="listbox"
                  aria-expanded={periodMenuOpen()}
                  aria-label="Time saved period"
                >
                  <span>{periodLabel()}</span>
                  <svg
                    width="10"
                    height="10"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    aria-hidden="true"
                  >
                    <polyline points="6 9 12 15 18 9" />
                  </svg>
                </button>
              </div>
              <Show when={periodMenuOpen()}>
                <div
                  ref={(el) => (periodMenuRef = el)}
                  class="liquid-glass z-50 max-h-60 overflow-y-auto rounded-[10px] py-1 scrollbar-thin"
                  role="listbox"
                  style={{
                    position: "fixed",
                    top: `${periodMenuPos().top}px`,
                    left: `${periodMenuPos().left}px`,
                    "min-width": `${periodMenuPos().minWidth}px`,
                  }}
                >
                  <For each={PERIOD_OPTIONS}>
                    {(o) => (
                      <button
                        type="button"
                        role="option"
                        aria-selected={o.value === period()}
                        class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] hover:bg-[var(--color-overlay)]"
                        classList={{
                          "text-[var(--color-accent)]": o.value === period(),
                          "text-[var(--color-ink)]": o.value !== period(),
                        }}
                        onClick={() => {
                          setPeriod(o.value);
                          setPeriodMenuOpen(false);
                          void refreshStats();
                        }}
                      >
                        <span class="flex-1">{o.label}</span>
                        <Show when={o.value === period()}>
                          <span class="text-[10px]">●</span>
                        </Show>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>

            <div class="border-b border-[var(--color-border)]">
              <button
                type="button"
                class="flex w-full items-center justify-between px-4 py-3 text-left transition-colors hover:bg-[var(--color-overlay)]"
                aria-expanded={quickSettingsOpen()}
                aria-controls="quick-settings-panel"
                onClick={() => setQuickSettingsOpen(!quickSettingsOpen())}
              >
                <span class="text-[13px] font-semibold text-[var(--color-ink)]">
                  Quick Settings
                </span>
                <svg
                  width="12"
                  height="12"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  class="text-[var(--color-ink-mute)] transition-transform duration-200 ease-out"
                  style={{
                    transform: quickSettingsOpen()
                      ? "rotate(180deg)"
                      : "rotate(0deg)",
                  }}
                  aria-hidden="true"
                >
                  <polyline points="6 9 12 15 18 9" />
                </svg>
              </button>
              <div
                id="quick-settings-panel"
                class="overflow-hidden transition-all duration-200 ease-out"
                style={{
                  "max-height": quickSettingsOpen() ? "500px" : "0px",
                  opacity: quickSettingsOpen() ? 1 : 0,
                }}
              >
                <div class="border-t border-[var(--color-border)] px-4 py-3">
                  <div class="flex items-center justify-between gap-3">
                    <div class="flex min-w-0 flex-col">
                      <span class="text-[13px] font-semibold text-[var(--color-ink)]">
                        {st().hands_free_enabled ? "Press to talk" : "Hold to talk"}
                      </span>
                      <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
                        {st().hands_free_enabled
                          ? "Press the hotkey to start, press again to stop."
                          : "Hold the hotkey while speaking, release to stop."}
                      </span>
                    </div>
                    <Toggle
                      checked={st().hands_free_enabled}
                      onChange={toggleHandsFree}
                    />
                  </div>
                </div>

                <div class="border-t border-[var(--color-border)] px-4 py-3">
                  <div class="flex items-center justify-between gap-3">
                    <div class="flex min-w-0 flex-col">
                      <span class="text-[13px] font-semibold text-[var(--color-ink)]">
                        Polish transcripts
                      </span>
                      <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
                        {st().polishing_enabled
                          ? "Gemma cleans up the transcript before copying."
                          : "Raw output is copied as-is."}
                      </span>
                    </div>
                    <Toggle
                      checked={st().polishing_enabled}
                      onChange={togglePolishing}
                    />
                  </div>
                  <Show
                    when={
                      st().polishing_enabled &&
                      st().llm_engine === "gemma-local"
                    }
                  >
                    <div class="mt-3 flex items-center justify-between gap-3 pl-6">
                      <div class="flex min-w-0 flex-col">
                        <span class="text-[12px] font-medium text-[var(--color-ink)]">
                          Thinking
                        </span>
                        <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
                          Reason silently before polishing.
                        </span>
                      </div>
                      <Toggle
                        checked={st().enable_thinking}
                        onChange={(v) => void toggleEnableThinking(v)}
                      />
                    </div>
                  </Show>
                </div>

                <div class="border-t border-[var(--color-border)] px-4 py-3">
                  <div class="flex items-center justify-between gap-3">
                    <div class="flex min-w-0 flex-col">
                      <span class="text-[13px] font-semibold text-[var(--color-ink)]">
                        Streaming pill
                      </span>
                      <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
                        See the transcript appear live before it's copied.
                      </span>
                    </div>
                    <Toggle
                      checked={st().streaming_pill_enabled}
                      onChange={(v) => void toggleStreamingPill(v)}
                    />
                  </div>
                </div>

                <div class="border-t border-[var(--color-border)] px-4 py-3">
                  <div class="flex items-center justify-between gap-3">
                    <div class="flex min-w-0 flex-col">
                      <span class="text-[13px] font-semibold text-[var(--color-ink)]">
                        Paste into focused field
                      </span>
                      <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
                        Vox sends ⌘V for you and restores your clipboard.
                      </span>
                    </div>
                    <Toggle
                      checked={st().auto_paste_enabled}
                      onChange={(v) => void toggleAutoPaste(v)}
                    />
                  </div>
                  <Show
                    when={
                      st().auto_paste_enabled && accessibility() === false
                    }
                  >
                    <div class="mt-2 flex items-center justify-between gap-3 rounded-[6px] bg-[var(--color-overlay)] px-3 py-2">
                      <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
                        Needs Accessibility. Falls back to clipboard otherwise.
                      </span>
                      <button
                        type="button"
                        class="shrink-0 rounded-[6px] bg-[var(--color-control)] px-2.5 py-1 text-[11px] font-medium text-[var(--color-ink)] hover:bg-[var(--color-control-hover)]"
                        onClick={() => void openAccessibilitySettings()}
                      >
                        Grant
                      </button>
                    </div>
                  </Show>
                </div>
              </div>
            </div>

            <div class="border-b border-[var(--color-border)] px-4 py-3">
              <div class="flex items-center justify-between gap-3">
                <div class="flex min-w-0 flex-col">
                  <span class="text-[13px] font-semibold text-[var(--color-ink)]">
                    Press to talk
                  </span>
                  <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
                    Trigger with{" "}
                    <span class="keycap">
                      {hotkeyLabel(st().hotkey || "CommandOrControl+Alt+Period")}
                    </span>
                  </span>
                </div>
              </div>
            </div>

            <div class="relative border-b border-[var(--color-border)] px-4 py-3">
              <div class="flex items-center justify-between gap-3">
                <div class="flex min-w-0 flex-col">
                  <span class="text-[13px] font-semibold text-[var(--color-ink)]">
                    Voice mode
                  </span>
                  <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
                    Cycle with{" "}
                    <span class="keycap">
                      {hotkeyLabel(st().mode_cycle_hotkey || "Alt+M")}
                    </span>
                  </span>
                </div>
                <button
                  ref={(el) => (modeTriggerRef = el)}
                  type="button"
                  class="liquid-card inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[12px] font-medium text-[var(--color-ink)] ring-1 ring-[var(--color-border-strong)] transition-[filter] hover:brightness-105"
                  onClick={() => {
                    const next = !modeMenuOpen();
                    if (next && modeTriggerRef) {
                      // The popover root has `overflow-hidden` so an
                      // `absolute` dropdown would clip inside the
                      // popover. Anchor via fixed coords from the
                      // trigger's bounding rect so the listbox can
                      // escape the clipped container without bringing
                      // a positioning library along for the ride.
                      const r = modeTriggerRef.getBoundingClientRect();
                      const menuW = Math.max(r.width, 224);
                      const right = r.right;
                      const left = Math.max(8, right - menuW);
                      setModeMenuPos({
                        top: r.bottom + 4,
                        left,
                        minWidth: menuW,
                      });
                    }
                    setModeMenuOpen(next);
                  }}
                  aria-haspopup="listbox"
                  aria-expanded={modeMenuOpen()}
                >
                  <Show when={activeMode()} fallback={<span>—</span>}>
                    {(m) => (
                      <>
                        <span aria-hidden="true">{m().icon}</span>
                        <span class="max-w-[120px] truncate">{m().name}</span>
                      </>
                    )}
                  </Show>
                  <svg
                    width="10"
                    height="10"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    aria-hidden="true"
                  >
                    <polyline points="6 9 12 15 18 9" />
                  </svg>
                </button>
              </div>
              <Show when={modeMenuOpen()}>
                <div
                  ref={(el) => (modeMenuRef = el)}
                  class="liquid-glass z-50 max-h-60 overflow-y-auto rounded-[10px] py-1 scrollbar-thin"
                  role="listbox"
                  style={{
                    position: "fixed",
                    top: `${modeMenuPos().top}px`,
                    left: `${modeMenuPos().left}px`,
                    "min-width": `${modeMenuPos().minWidth}px`,
                  }}
                >
                  <For each={modes()}>
                    {(m) => (
                      <button
                        type="button"
                        role="option"
                        aria-selected={m.id === st().active_mode_id}
                        class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] hover:bg-[var(--color-overlay)]"
                        classList={{
                          "text-[var(--color-accent)]":
                            m.id === st().active_mode_id,
                          "text-[var(--color-ink)]":
                            m.id !== st().active_mode_id,
                        }}
                        onClick={() => pickMode(m.id)}
                      >
                        <span aria-hidden="true">{m.icon}</span>
                        <span class="flex-1 truncate">{m.name}</span>
                        <Show when={m.id === st().active_mode_id}>
                          <span class="text-[10px]">●</span>
                        </Show>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>

            <div class="relative border-b border-[var(--color-border)] px-4 py-3">
              <div class="flex items-center justify-between gap-3">
                <div class="flex min-w-0 flex-col">
                  <span class="text-[13px] font-semibold text-[var(--color-ink)]">
                    Microphone
                  </span>
                  <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
                    {micIsCustom()
                      ? "Custom input selected."
                      : "Follows the macOS Sound input."}
                  </span>
                </div>
                <button
                  ref={(el) => (micTriggerRef = el)}
                  type="button"
                  class="liquid-card inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[12px] font-medium text-[var(--color-ink)] ring-1 ring-[var(--color-border-strong)] transition-[filter] hover:brightness-105"
                  onClick={() => {
                    const next = !micMenuOpen();
                    if (next && micTriggerRef) {
                      // Same fixed-position trick as the mode picker
                      // so the dropdown can escape the popover root's
                      // overflow-hidden. Cap the width at 280px so a
                      // long device name doesn't stretch the menu
                      // wider than the popover itself. Flip the menu
                      // above the trigger when there's more headroom
                      // up there — the Mic row lives mid-popover so a
                      // 6-device list otherwise clips at the bottom
                      // window edge.
                      const r = micTriggerRef.getBoundingClientRect();
                      const viewportH = window.innerHeight;
                      const menuW = Math.min(280, Math.max(r.width, 240));
                      const left = Math.max(8, r.right - menuW);
                      const spaceBelow = viewportH - r.bottom - 8;
                      const spaceAbove = r.top - 8;
                      // ~"Follow system default" + divider + 6 rows.
                      const desiredH = 220;
                      const placeBelow =
                        spaceBelow >= desiredH || spaceBelow >= spaceAbove;
                      if (placeBelow) {
                        setMicMenuPos({
                          placement: "below",
                          top: r.bottom + 4,
                          bottom: 0,
                          left,
                          minWidth: menuW,
                          maxHeight: spaceBelow,
                        });
                      } else {
                        setMicMenuPos({
                          placement: "above",
                          top: 0,
                          bottom: viewportH - r.top + 4,
                          left,
                          minWidth: menuW,
                          maxHeight: spaceAbove,
                        });
                      }
                    }
                    setMicMenuOpen(next);
                  }}
                  aria-haspopup="listbox"
                  aria-expanded={micMenuOpen()}
                  title={micLabel()}
                >
                  <span class="max-w-[140px] truncate">{micLabel()}</span>
                  <svg
                    width="10"
                    height="10"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    aria-hidden="true"
                  >
                    <polyline points="6 9 12 15 18 9" />
                  </svg>
                </button>
              </div>
              <Show when={micMenuOpen()}>
                <div
                  ref={(el) => (micMenuRef = el)}
                  class="liquid-glass z-50 overflow-y-auto rounded-[10px] py-1 scrollbar-thin"
                  role="listbox"
                  aria-label="Audio input device"
                  style={{
                    position: "fixed",
                    ...(micMenuPos().placement === "above"
                      ? { bottom: `${micMenuPos().bottom}px` }
                      : { top: `${micMenuPos().top}px` }),
                    left: `${micMenuPos().left}px`,
                    "min-width": `${micMenuPos().minWidth}px`,
                    "max-height": `${micMenuPos().maxHeight}px`,
                  }}
                >
                  <button
                    type="button"
                    role="option"
                    aria-selected={!micIsCustom()}
                    class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] hover:bg-[var(--color-overlay)]"
                    classList={{
                      "text-[var(--color-accent)]": !micIsCustom(),
                      "text-[var(--color-ink)]": micIsCustom(),
                    }}
                    onClick={() => void pickMic(null)}
                  >
                    <span class="flex-1 truncate">Follow system default</span>
                    <Show when={!micIsCustom()}>
                      <span class="text-[10px]">●</span>
                    </Show>
                  </button>
                  <Show when={audioDevices().length > 0}>
                    <div class="my-1 mx-2 h-px bg-[var(--color-border)]" />
                  </Show>
                  <For each={audioDevices()}>
                    {(d) => {
                      const active = () => st().audio_input_device === d.name;
                      return (
                        <button
                          type="button"
                          role="option"
                          aria-selected={active()}
                          class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] hover:bg-[var(--color-overlay)]"
                          classList={{
                            "text-[var(--color-accent)]": active(),
                            "text-[var(--color-ink)]": !active(),
                          }}
                          onClick={() => void pickMic(d.name)}
                          title={d.name}
                        >
                          <span class="flex-1 truncate">
                            {d.name}
                            {d.is_default ? " · system default" : ""}
                          </span>
                          <Show when={active()}>
                            <span class="text-[10px]">●</span>
                          </Show>
                        </button>
                      );
                    }}
                  </For>
                  <Show
                    when={
                      !!st().audio_input_device &&
                      !audioDevices().some(
                        (d) => d.name === st().audio_input_device,
                      )
                    }
                  >
                    <div class="my-1 mx-2 h-px bg-[var(--color-border)]" />
                    <div class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[11px] text-[var(--color-ink-mute)]">
                      <span
                        class="flex-1 truncate"
                        title={st().audio_input_device ?? ""}
                      >
                        {st().audio_input_device} · not connected
                      </span>
                    </div>
                  </Show>
                </div>
              </Show>
            </div>

            <div class="flex min-h-[180px] flex-1 flex-col px-2 py-2">
              <Show when={inProgress().length > 0}>
                <div class="mb-1 px-2 text-[10px] font-semibold uppercase tracking-[0.06em] text-[var(--color-violet,#a78bfa)]">
                  In progress
                </div>
                <For each={inProgress()}>
                  {(entry) => (
                    <InProgressRow
                      entry={entry}
                      onCancel={() => {
                        void cancelSession(entry.session_id);
                      }}
                    />
                  )}
                </For>
                <div class="mb-2 mt-2 h-px bg-[var(--color-border)]" />
              </Show>
              <Show
                when={recents().length > 0}
                fallback={
                  <>
                    <div class="mb-1 px-2 text-[10px] font-semibold uppercase tracking-[0.06em] text-[var(--color-ink-faint)]">
                      Recent
                    </div>
                    <div class="px-2 py-4 text-[12px] text-[var(--color-ink-mute)]">
                      Your last 5 transcripts will appear here.
                    </div>
                  </>
                }
              >
                <div class="mb-1 px-2 text-[10px] font-semibold uppercase tracking-[0.06em] text-[var(--color-ink-faint)]">
                  Recent (cleared on quit)
                </div>
                <For each={recents()}>
                  {(r, idx) => {
                    const busy = () => repolishBusy().has(r.session_id);
                    const flash = () => {
                      const f = repolishFlash();
                      return f && f.sessionId === r.session_id ? f : null;
                    };
                    const err = () => {
                      const e = repolishError();
                      return e && e.sessionId === r.session_id ? e : null;
                    };
                    return (
                      <div class="group flex flex-col gap-1 rounded-[8px] px-2 py-2 transition-colors hover:bg-[var(--color-overlay)]">
                        <div class="flex items-center gap-2">
                          <span class="w-10 shrink-0 text-[11px] font-medium text-[var(--color-ink-faint)]">
                            {ago(r.ts)}
                          </span>
                          <span class="flex-1 truncate text-[12px] text-[var(--color-ink)]">
                            {r.text}
                          </span>
                          <button
                            ref={(el) => {
                              if (el) repolishTriggerRefs.set(r.session_id, el);
                              else repolishTriggerRefs.delete(r.session_id);
                            }}
                            type="button"
                            disabled={busy()}
                            class="repolish-trigger inline-flex h-7 w-7 items-center justify-center rounded p-1.5 text-[var(--color-ink-mute)] opacity-0 transition-opacity duration-150 ease-out hover:bg-[var(--color-overlay)] hover:text-[var(--color-accent)] group-hover:opacity-100 focus-visible:opacity-100 disabled:cursor-not-allowed disabled:opacity-100"
                            classList={{
                              "!opacity-100":
                                busy() ||
                                repolishPickerSessionId() === r.session_id,
                            }}
                            aria-haspopup="listbox"
                            aria-expanded={
                              repolishPickerSessionId() === r.session_id
                            }
                            aria-label={`Re-polish as another mode`}
                            title="Re-polish as…"
                            onClick={(e) => {
                              e.stopPropagation();
                              if (busy()) return;
                              if (repolishPickerSessionId() === r.session_id) {
                                setRepolishPickerSessionId(null);
                              } else {
                                openRepolishPicker(r.session_id);
                              }
                            }}
                          >
                            <Show
                              when={!busy()}
                              fallback={<SmallSpinner />}
                            >
                              <SparkleGlyph />
                            </Show>
                          </button>
                          <button
                            type="button"
                            class="inline-flex h-7 w-7 items-center justify-center rounded p-1.5 text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]"
                            onClick={() => copyRecent(idx(), r.text)}
                            title="Copy to clipboard (⌘V to paste)"
                          >
                            <Show
                              when={copyFlash() === idx()}
                              fallback={
                                <svg
                                  width="14"
                                  height="14"
                                  viewBox="0 0 24 24"
                                  fill="none"
                                  stroke="currentColor"
                                  stroke-width="1.75"
                                  stroke-linecap="round"
                                  stroke-linejoin="round"
                                >
                                  <rect x="9" y="9" width="12" height="12" rx="2" />
                                  <path d="M5 15V5a2 2 0 0 1 2-2h10" />
                                </svg>
                              }
                            >
                              <span class="text-[var(--color-success)] text-[11px]">
                                ✓
                              </span>
                            </Show>
                          </button>
                        </div>
                        <Show when={flash()}>
                          {(f) => (
                            <div
                              class="ml-12 flex items-center gap-1 text-[10.5px] leading-none transition-opacity duration-150 ease-out"
                              classList={{
                                "text-[var(--color-success)]": !f().warning,
                                "text-[var(--color-warning)]": f().warning,
                              }}
                            >
                              <span aria-hidden="true">
                                {f().warning ? "!" : "✓"}
                              </span>
                              <span>
                                {f().warning
                                  ? `Raw copy — ${f().modeName} polish failed`
                                  : `Re-polished as ${f().modeName} — copied`}
                              </span>
                            </div>
                          )}
                        </Show>
                        <Show when={err()}>
                          {(e) => (
                            <div class="ml-12 flex items-center gap-1 text-[10.5px] leading-none text-[var(--color-danger)]">
                              <span aria-hidden="true">!</span>
                              <span>{e().message}</span>
                            </div>
                          )}
                        </Show>
                      </div>
                    );
                  }}
                </For>
              </Show>
            </div>

            <Show when={repolishPickerSessionId() !== null}>
              <div
                ref={(el) => (repolishPickerRef = el)}
                class="liquid-glass z-50 overflow-y-auto rounded-[10px] py-1 scrollbar-thin"
                role="listbox"
                aria-label="Re-polish with mode"
                style={{
                  position: "fixed",
                  ...(repolishMenuPos().placement === "above"
                    ? { bottom: `${repolishMenuPos().bottom}px` }
                    : { top: `${repolishMenuPos().top}px` }),
                  left: `${repolishMenuPos().left}px`,
                  "min-width": `${repolishMenuPos().minWidth}px`,
                  "max-height": `${repolishMenuPos().maxHeight}px`,
                }}
              >
                <div class="px-3 pb-1 pt-1.5 text-[10px] font-semibold uppercase tracking-[0.06em] text-[var(--color-ink-faint)]">
                  Re-polish as…
                </div>
                <For each={modes()}>
                  {(m) => {
                    const target = repolishPickerSessionId();
                    const targetEntry = () =>
                      recents().find((r) => r.session_id === target);
                    const isCurrent = () => targetEntry()?.mode_id === m.id;
                    return (
                      <button
                        type="button"
                        role="option"
                        aria-selected={isCurrent()}
                        class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] hover:bg-[var(--color-overlay)]"
                        classList={{
                          "text-[var(--color-accent)]": isCurrent(),
                          "text-[var(--color-ink)]": !isCurrent(),
                        }}
                        onClick={() => {
                          const sid = repolishPickerSessionId();
                          if (sid !== null) void runRepolish(sid, m.id);
                        }}
                      >
                        <span aria-hidden="true">{m.icon}</span>
                        <span class="flex-1 truncate">{m.name}</span>
                        <Show when={isCurrent()}>
                          <span class="text-[10px]">●</span>
                        </Show>
                      </button>
                    );
                  }}
                </For>
              </div>
            </Show>

            </div>
            <div class="flex shrink-0 items-center justify-between border-t border-[var(--color-border)] px-4 py-3">
              <span class="text-[12px] font-medium text-[var(--color-ink-mute)]">
                Appearance
              </span>
              <AppearanceToggle
                value={st().appearance}
                onChange={(v) => {
                  const cur = s();
                  if (cur) setS({ ...cur, appearance: v });
                  void setAppearance(v);
                }}
              />
            </div>
            <footer class="flex shrink-0 items-center justify-between gap-2 border-t border-[var(--color-border)] px-4 py-2.5">
              <LocalBadge />
              <button
                type="button"
                class="rounded-[6px] px-3 py-1.5 text-[12px] font-medium text-[var(--color-ink-mute)] transition-colors hover:bg-[var(--color-overlay)] hover:text-[var(--color-danger)]"
                onClick={() => quitApp()}
              >
                Quit
              </button>
            </footer>
          </>
        )}
      </Show>
    </div>
  );
}

function StatusDot(props: { tone: "ok" | "off" | "warn" }) {
  const color =
    props.tone === "ok"
      ? "var(--color-success)"
      : props.tone === "warn"
        ? "var(--color-warning)"
        : "var(--color-ink-faint)";
  return (
    <span
      class="inline-block h-2 w-2 rounded-full"
      style={{ background: color }}
    />
  );
}

function stageLabel(stage: Stage): string {
  switch (stage) {
    case "listening":
      return "Listening…";
    case "transcribing":
      return "Transcribing…";
    case "polishing":
      return "Polishing…";
    case "done":
      return "Done";
    case "error":
      return "Error";
    case "canceled":
      return "Canceled";
    default:
      return "…";
  }
}

function InProgressRow(props: {
  entry: InProgressEntry;
  onCancel: () => void;
}) {
  return (
    <div class="group flex items-center gap-2 rounded-[8px] px-2 py-2 transition-colors hover:bg-[var(--color-overlay)]">
      <span aria-hidden="true" class="text-[14px] leading-none">
        {props.entry.mode_icon}
      </span>
      <Spinner />
      <span
        class="pill-shimmer flex-1 truncate text-[12px] font-medium leading-none"
        style={{
          "--shimmer-base": "var(--color-ink-mute)",
          "--shimmer-tint": "var(--color-violet, #a78bfa)",
          "letter-spacing": "0.01em",
        }}
      >
        {stageLabel(props.entry.stage)}
      </span>
      <span class="shrink-0 text-[11px] text-[var(--color-ink-faint)]">
        {props.entry.mode_name}
      </span>
      <button
        type="button"
        class="inline-flex h-7 w-7 items-center justify-center rounded p-1.5 text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-danger)]"
        title="Cancel session"
        aria-label="Cancel session"
        onClick={(e) => {
          e.stopPropagation();
          props.onCancel();
        }}
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <line x1="6" y1="6" x2="18" y2="18" />
          <line x1="6" y1="18" x2="18" y2="6" />
        </svg>
      </button>
    </div>
  );
}

function Spinner() {
  return (
    <svg
      class="animate-spin"
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-violet, #a78bfa)"
      stroke-width="2.5"
      stroke-linecap="round"
      aria-hidden="true"
    >
      <path d="M12 3a9 9 0 1 0 9 9" />
    </svg>
  );
}

function SmallSpinner() {
  // Sized to match the 14×14 svgs already used by the row actions so
  // the spinner slots in without shifting layout.
  return (
    <svg
      class="animate-spin"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--color-accent)"
      stroke-width="2.5"
      stroke-linecap="round"
      aria-hidden="true"
    >
      <path d="M12 3a9 9 0 1 0 9 9" />
    </svg>
  );
}

function SparkleGlyph() {
  // Liquid-style 4-point sparkle — reads as "polish / refresh" without
  // looking like a copy or refresh-arrow glyph (both of which already
  // appear elsewhere in the popover).
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M12 3v4M12 17v4M3 12h4M17 12h4" />
      <path d="M12 8c.5 2 1.5 3 4 3.5C13.5 12 12.5 13 12 16c-.5-3-1.5-4-4-4.5C10.5 11 11.5 10 12 8z" />
    </svg>
  );
}

function shortRepolishError(message: string): string {
  // The backend returns string errors from `Result<_, String>`. Trim
  // boilerplate (`anyhow` chain, IPC framing) for the dense popover.
  const trimmed = message.replace(/^Error:\s*/i, "").trim();
  if (trimmed.length <= 80) return trimmed;
  return trimmed.slice(0, 77) + "…";
}

function LocalBadge() {
  // Quiet reminder, no popover, no tooltip on the badge itself —
  // if the user cares they'll click through to the Privacy tab.
  // DESIGN.md §5.5.
  return (
    <button
      type="button"
      class="inline-flex items-center gap-1.5 rounded-full px-2 py-1 text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--color-ink-mute)] transition-colors hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]"
      onClick={() => openSettingsWindow()}
      title="Audio never leaves this Mac"
    >
      <svg
        width="11"
        height="11"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M12 21s-7-4-7-10V6l7-3 7 3v5c0 6-7 10-7 10z" />
      </svg>
      <span>Local</span>
    </button>
  );
}

function Toggle(props: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.checked}
      onClick={() => props.onChange(!props.checked)}
      class="relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full ring-1 ring-inset transition-colors"
      classList={{
        "bg-[var(--color-accent)] ring-transparent": props.checked,
        "bg-[var(--color-control)] ring-[var(--color-border-strong)]":
          !props.checked,
      }}
    >
      <span
        class="toggle-knob inline-block h-[18px] w-[18px] transform rounded-full transition-transform"
        classList={{
          "translate-x-[18px]": props.checked,
          "translate-x-[2px]": !props.checked,
        }}
      />
    </button>
  );
}

function AppearanceToggle(props: {
  value: "light" | "dark";
  onChange: (v: "light" | "dark") => void;
}) {
  return (
    <div
      class="inline-flex items-center rounded-full p-[3px] ring-1 ring-inset ring-[var(--color-border)]"
      style={{ background: "var(--color-control)" }}
      role="radiogroup"
      aria-label="Appearance"
    >
      <AppearanceHalf
        active={props.value === "light"}
        label="Light"
        onClick={() => props.onChange("light")}
      >
        <SunGlyph />
      </AppearanceHalf>
      <AppearanceHalf
        active={props.value === "dark"}
        label="Dark"
        onClick={() => props.onChange("dark")}
      >
        <MoonGlyph />
      </AppearanceHalf>
    </div>
  );
}

function AppearanceHalf(props: {
  active: boolean;
  label: string;
  onClick: () => void;
  children: any;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={props.active}
      aria-label={props.label}
      class="inline-flex h-6 w-7 items-center justify-center rounded-full transition-all"
      classList={{
        "hover:text-[var(--color-ink)]": !props.active,
      }}
      style={{
        background: props.active ? "var(--color-surface-raised)" : "transparent",
        "backdrop-filter": props.active ? "var(--glass-blur)" : "none",
        "-webkit-backdrop-filter": props.active ? "var(--glass-blur)" : "none",
        "box-shadow": props.active ? "var(--glass-shadow-stack-sm)" : "none",
        color: props.active
          ? "var(--color-ink)"
          : "var(--color-ink-mute)",
      }}
      onClick={props.onClick}
    >
      {props.children}
    </button>
  );
}

function SunGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="3.5" />
      <line x1="12" y1="2" x2="12" y2="4" />
      <line x1="12" y1="20" x2="12" y2="22" />
      <line x1="2" y1="12" x2="4" y2="12" />
      <line x1="20" y1="12" x2="22" y2="12" />
      <line x1="4.9" y1="4.9" x2="6.3" y2="6.3" />
      <line x1="17.7" y1="17.7" x2="19.1" y2="19.1" />
      <line x1="4.9" y1="19.1" x2="6.3" y2="17.7" />
      <line x1="17.7" y1="6.3" x2="19.1" y2="4.9" />
    </svg>
  );
}

function MoonGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" />
    </svg>
  );
}

function formatDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds)) return "0s";
  const t = Math.round(totalSeconds);
  if (t < 1) return "0s";
  if (t < 60) return `${t}s`;
  if (t < 3600) {
    const m = Math.floor(t / 60);
    const s = t - m * 60;
    return s === 0 ? `${m}m` : `${m}m ${s}s`;
  }
  const h = Math.floor(t / 3600);
  const m = Math.floor((t - h * 3600) / 60);
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

function sessionsSubLine(n: number): string {
  if (n === 0) return "No sessions yet";
  if (n === 1) return "1 session";
  return `${n} sessions`;
}

function ago(tsSeconds: number): string {
  const now = Date.now() / 1000;
  const delta = Math.max(0, Math.floor(now - tsSeconds));
  if (delta < 60) return `${delta}s`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m`;
  if (delta < 86_400) return `${Math.floor(delta / 3600)}h`;
  return `${Math.floor(delta / 86_400)}d`;
}

function hotkeyLabel(k: string): string {
  switch (k) {
    case "F13":
      return "F13";
    case "F14":
      return "F14";
    case "F15":
      return "F15";
    case "Alt+M":
      return "⌥M";
    case "Alt+Shift+M":
      return "⌥⇧M";
    case "CommandOrControl+Shift+M":
    case "Cmd+Shift+M":
      return "⌘⇧M";
    case "CommandOrControl+Alt+Period":
    case "Cmd+Alt+Period":
      return "⌘⌥.";
    default:
      // Best-effort fallback so any user-configured chord at least
      // shows its modifier glyphs instead of the raw "Alt+Shift+V"
      // tokens. Mirrors the formatter in Onboarding.tsx without the
      // shifted-glyph collapsing — the popover is content with the
      // simpler rendering.
      return k
        .split("+")
        .map((t) => {
          const trimmed = t.trim();
          switch (trimmed) {
            case "Alt":
            case "Option":
              return "⌥";
            case "Shift":
              return "⇧";
            case "Cmd":
            case "Command":
            case "CommandOrControl":
            case "Meta":
            case "Super":
              return "⌘";
            case "Ctrl":
            case "Control":
              return "⌃";
            case "Period":
              return ".";
            case "Comma":
              return ",";
            case "Slash":
              return "/";
            case "Semicolon":
              return ";";
            case "Backslash":
              return "\\";
            case "Quote":
              return "'";
            default:
              return trimmed;
          }
        })
        .join("");
  }
}
