import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Settings {
  hotkey: string;
  cancel_hotkey: string;
  /// Require pressing the cancel hotkey (default Escape) twice quickly
  /// to abort a live recording, instead of cancelling on the first
  /// press. Off by default. Guards against a stray Escape canceling
  /// dictation; only the recording phase is gated.
  cancel_requires_double_press: boolean;
  launch_at_login: boolean;
  dictation_enabled: boolean;
  hands_free_enabled: boolean;
  language: string;
  onboarded: boolean;
  dictionary: string[];
  /// Free-form personal context (role, current projects, people the
  /// user mentions often) merged into the cleanup prompt of every mode
  /// whose `context_enabled` is true. Empty by default.
  context: string;
  stt_engine: string;
  stt_model_id: string;
  llm_engine: string;
  llm_model_id: string;
  /// Survives engine outages so user intent isn't lost while the local
  /// LLM is temporarily unavailable.
  polishing_enabled: boolean;
  gemma_tos_accepted: boolean;
  active_mode_id: string;
  mode_cycle_hotkey: string;
  auto_suggest_modes: boolean;
  appearance: "light" | "dark";
  /// Show a Dock icon (and ⌘-Tab entry) for Vox. Off by default — Vox is
  /// a menu-bar-only accessory app. When true, the activation policy is
  /// `Regular`; clicking the Dock icon opens Settings. The menu-bar icon
  /// is always present regardless.
  show_dock_icon: boolean;
  /// Show the transcript / polish text streaming inside the pill after
  /// the hotkey is released. Default true; tray popover + Settings →
  /// General both flip it.
  streaming_pill_enabled: boolean;
  /// Auto-paste the polished transcript into the focused text field by
  /// synthesising ⌘V at the end of the pipeline. Requires Accessibility
  /// permission; when the toggle is on but Accessibility isn't granted,
  /// the pipeline falls open to the clipboard-only path and the pill
  /// surfaces a warning. Off by default.
  auto_paste_enabled: boolean;
  /// What to do with other audio that's playing when dictation starts:
  /// `"off"` leaves it alone, `"duck"` lowers its volume, `"pause"`
  /// stops playback. Restored when dictation ends. Defaults to `"duck"`.
  audio_attenuation: "off" | "duck" | "pause";
  /// Show a live transcript inside the pill while the user is still
  /// speaking, in place of the default waveform. Driven by the same
  /// speculative transcription decoder that already feeds the post-stop
  /// reveal — flipping this on reveals what was previously buffered
  /// silently. Off by default.
  live_transcript_enabled: boolean;
  /// Let Gemma reason silently before polishing. Only affects
  /// `gemma-local`; ignored by Apple Intelligence and the no-op engine.
  /// Default false.
  enable_thinking: boolean;
  /// Token identifying our `llama-server` children for orphan reaping.
  /// Not a secret (server is loopback-only).
  inference_api_key: string;
  /// User dismissed the "Try Parakeet" upgrade banner in
  /// Settings → Models. Sticky once true.
  try_parakeet_dismissed: boolean;
  /// Name of the audio input device Vox should listen to. `null`
  /// (or an empty string) means follow the macOS system default —
  /// re-resolved every recording, so swapping inputs in System
  /// Settings takes effect on the next dictation.
  audio_input_device: string | null;
  /// Start a dictation session by speaking the wake phrase ("Hey Vox"
  /// by default). Keeps the microphone listening while Vox is idle —
  /// fully on-device, audio never leaves RAM. Off by default.
  wake_word_enabled: boolean;
  /// The spoken phrase that triggers a session when the voice trigger
  /// is on. Matched fuzzily so near-homophones still trigger; blank
  /// falls back to the default on the backend.
  wake_phrase: string;
  /// Notch List (macOS-only). Enabled by default (opt-out). When true
  /// on macOS, a top-center floating notch window shows the top to-do
  /// item and expands on hover. No window effect on Windows.
  notch_list_enabled: boolean;
  /// How many top items the notch panel reveals on hover (clamped 3–7).
  notch_list_top_n: number;
  /// Which display the notch attaches to.
  notch_list_display: "active" | "builtin" | "primary";
  /// Dedicated hotkey that routes the next dictation through the notch
  /// command interpreter instead of polish + clipboard. Default ⌥⌘L.
  notch_list_voice_hotkey: string;
  /// Stage destructive voice batches (remove/clear) for confirmation
  /// rather than applying immediately. On by default.
  notch_list_confirm_destructive: boolean;
  /// Intent-delay in milliseconds before the notch panel expands on
  /// hover (clamped 0–1000). UI-only.
  notch_list_hover_delay_ms: number;
  /// When true, the notch expands on click instead of hover. UI-only.
  notch_list_click_to_expand: boolean;
  /// Periodic auto-update check. On by default. When off, the
  /// background scheduler in `updater.rs` is a no-op but the manual
  /// "Check now" button in Settings → About still works.
  auto_update_check_enabled: boolean;
  /// Unix epoch seconds of the most recent SUCCESSFUL update check.
  /// `null` until the first check lands. Used in Settings → About
  /// to surface "Last checked X ago" beside the manual button.
  last_update_check_at?: number | null;
  /// Version the user "Later"-dismissed in the tray banner. While
  /// this equals the current available version, the tray banner +
  /// tray-icon dot stay hidden. Settings → About still surfaces the
  /// pending state regardless.
  dismissed_update_version?: string | null;
  /// Version for which the prominent auto-show update window has
  /// already been surfaced. Set once it builds so the window fires at
  /// most once per available version. Independent of the tray banner.
  update_window_shown_version?: string | null;
}

export interface AudioInputDevice {
  name: string;
  is_default: boolean;
}

export type PunctuationStyle = "Aggressive" | "Standard" | "Minimal" | "None";
export type CaseStyle = "Sentence" | "Title" | "Lower" | "AsSpoken";

export interface AutoTrigger {
  bundle_id_glob?: string | null;
  window_title_substring?: string | null;
}

export interface VoiceMode {
  id: string;
  name: string;
  icon: string;
  description: string;
  system_prompt: string;
  dictionary: string[];
  punctuation: PunctuationStyle;
  case: CaseStyle;
  max_length_chars?: number | null;
  /// Tri-state: `null`/missing inherits the global `enable_thinking`,
  /// `true`/`false` overrides it. Only affects `gemma-local`.
  enable_thinking?: boolean | null;
  /// Whether the global personal context (Settings → Dictionary &
  /// Context) is folded into this mode's cleanup prompt. Backend always
  /// serializes it; defaults to `true`.
  context_enabled: boolean;
  auto_triggers: AutoTrigger[];
  built_in: boolean;
  created_at: number;
  updated_at: number;
}

// --- Notch List -----------------------------------------------------------

export type NotchPriority = "high" | "med" | "low";

export interface NotchItem {
  id: string;
  text: string;
  priority: NotchPriority;
  order: number;
  done: boolean;
  created_at: number;
  completed_at?: number | null;
}

export interface NotchListDoc {
  version: number;
  items: NotchItem[];
  archive: NotchItem[];
}

export async function getNotchList(): Promise<NotchListDoc> {
  return invoke<NotchListDoc>("notch_list_get");
}

/// The only notch-list mutation callable from the notch window itself.
export async function notchListMarkDone(id: string): Promise<void> {
  await invoke("notch_list_mark_done", { id });
}

export async function notchListAdd(
  text: string,
  priority?: NotchPriority,
): Promise<NotchItem> {
  return invoke<NotchItem>("notch_list_add", { text, priority: priority ?? null });
}

export async function notchListEdit(id: string, text: string): Promise<void> {
  await invoke("notch_list_edit", { id, text });
}

export async function notchListRemove(id: string): Promise<void> {
  await invoke("notch_list_remove", { id });
}

export async function notchListReorder(orderedIds: string[]): Promise<void> {
  await invoke("notch_list_reorder", { orderedIds });
}

export async function notchListSetPriority(
  id: string,
  priority: NotchPriority,
): Promise<void> {
  await invoke("notch_list_set_priority", { id, priority });
}

export async function notchListRestore(id: string): Promise<void> {
  await invoke("notch_list_restore", { id });
}

export async function notchListClearCompleted(): Promise<void> {
  await invoke("notch_list_clear_completed");
}

export async function notchListUndo(): Promise<boolean> {
  return invoke<boolean>("notch_list_undo");
}

/// Resize + re-center the notch window (hover expand / collapse).
export async function notchResize(
  width: number | null,
  height: number | null,
): Promise<void> {
  await invoke("notch_resize", { width, height });
}

/// Fired by the backend whenever the notch list changes (any mutation,
/// including voice commands). Payload is the full current doc. Subscribed
/// by the notch window and the Settings → Notch List tab so both stay in
/// sync without polling.
export async function onNotchListChanged(
  cb: (doc: NotchListDoc) => void,
): Promise<UnlistenFn> {
  return listen<NotchListDoc>("notch-list:changed", (e) => cb(e.payload));
}

/// Physical-notch geometry (logical px) for the display the notch is
/// attached to. The notch webview sizes its idle "nub" to `width` so it
/// never spills past the camera bump, and uses `has_notch` to decide
/// whether it's hanging below a real notch. Zeroed on a notchless display.
export interface NotchGeometry {
  has_notch: boolean;
  width: number;
  height: number;
}

/// Snapshot the current notch geometry. Called by the notch webview on
/// mount; pair with `onNotchGeometry` for live updates.
export async function getNotchGeometry(): Promise<NotchGeometry> {
  return invoke<NotchGeometry>("notch_geometry");
}

/// Fired when the notch hops to a display with different notch geometry
/// (e.g. cursor-follow across monitors, or the notch first becoming known
/// after launch). Payload is the new geometry.
export async function onNotchGeometry(
  cb: (geo: NotchGeometry) => void,
): Promise<UnlistenFn> {
  return listen<NotchGeometry>("notch:geometry", (e) => cb(e.payload));
}

/// Cursor-over-notch hit-testing, pushed from Rust. The notch webview
/// can't observe the cursor while another app is focused (DOM mouse
/// events don't fire across the window boundary), so Rust hit-tests the
/// physical notch rect against the global cursor and emits `true` when
/// the cursor enters the notch area, `false` when it leaves. This is the
/// authoritative driver for expand-on-hover (the DOM `onMouseEnter` /
/// `onMouseLeave` handlers remain only as a secondary trigger).
export async function onNotchHover(
  cb: (over: boolean) => void,
): Promise<UnlistenFn> {
  return listen<boolean>("notch:hover", (e) => cb(e.payload));
}

/// Voice-capture phase for the in-notch ⌘L flow. `listening` = mic open,
/// show the waveform ribbon; `compiling` = interpreting the utterance,
/// show a pulsing indicator; `error` = surface `message` briefly; `idle`
/// = fade back to the resting collapsed/empty drawer.
export interface NotchListeningState {
  phase: "listening" | "compiling" | "idle" | "error";
  message?: string;
}

export async function onNotchListening(
  cb: (s: NotchListeningState) => void,
): Promise<UnlistenFn> {
  return listen<NotchListeningState>("notch:listening", (e) => cb(e.payload));
}

/// Live audio levels for the in-notch waveform. Length-32 array, each
/// value 0..1 — the same shape/cadence as the pill's `pill:levels`.
export async function onNotchLevels(
  cb: (levels: number[]) => void,
): Promise<UnlistenFn> {
  return listen<number[]>("notch:levels", (e) => cb(e.payload));
}

export interface UpdateCheckResult {
  status: "available" | "up-to-date" | "error" | "unconfigured";
  current_version?: string;
  available_version?: string;
  message?: string;
  /// `notes` field from the release's `latest.json` — typically a
  /// bullet list of commit subjects since the previous tag. Only
  /// populated when `status === "available"`.
  release_notes?: string;
}

/// Mirrors the Rust `updater::UpdateState` shape. Emitted on
/// `updates:state` whenever any field changes; also returned
/// synchronously by `getUpdateState()` for initial render.
/// `status: "checking"` is a transient state set while an HTTPS
/// round-trip is in flight.
export interface UpdateState {
  status: "idle" | "checking" | "available" | "up-to-date" | "error" | "unconfigured";
  current_version: string;
  available_version?: string | null;
  release_notes?: string | null;
  /// Unix epoch seconds. `null` until the first successful check.
  last_check_at?: number | null;
  /// Last error message from a failed check. Cleared on the next
  /// successful check.
  last_error?: string | null;
  /// Version the user "Later"-dismissed. While this equals
  /// `available_version`, the banner + dot stay hidden.
  dismissed_version?: string | null;
}

/// Mirrors the `UpdaterProgress` enum emitted on `updater:progress`.
/// `download_and_install_update` fires these as it walks: started →
/// progress* → installing → done. An `error` payload terminates the
/// sequence.
export type UpdaterProgress =
  | { phase: "started"; content_length?: number | null }
  | {
      phase: "progress";
      downloaded: number;
      content_length?: number | null;
    }
  | { phase: "installing" }
  | { phase: "done" }
  | { phase: "error"; message: string };

export interface RecentTranscript {
  ts: number;
  text: string;
}

export type Period = "last_session" | "day" | "week" | "month" | "year";

export interface TimeSavedStats {
  saved_seconds: number;
  sessions: number;
}

export interface DonationPrompt {
  /// The milestone tier (seconds) that triggered this nudge: 3600 (1h),
  /// 36000 (10h), or 180000 (50h). Passed back to `donationSnooze` so the
  /// shown tier is the one recorded as dismissed.
  milestone_secs: number;
  /// Cumulative seconds saved so far — drives the value-quantified copy.
  saved_secs: number;
}

export type Stage =
  | "listening"
  | "transcribing"
  | "polishing"
  | "done"
  | "error"
  | "canceled";

export interface InProgressEntry {
  session_id: number;
  stage: Stage;
  mode_id: string;
  mode_name: string;
  mode_icon: string;
  started_at: number;
}

export interface CompletedEntry {
  session_id: number;
  ts: number;
  text: string;
  /**
   * Pre-polish Whisper transcript captured at dictation time. Used as
   * the input for `rePolishTranscript`. Absent for entries that
   * pre-date this field, or for any entry that fell through the LLM
   * fail-open (in which case `text` is already the raw transcript).
   */
  raw_text?: string | null;
  mode_id: string;
  stage: Stage;
  warning: boolean;
  error_message: string | null;
}

export interface SessionsSnapshot {
  in_progress: InProgressEntry[];
  recent: CompletedEntry[];
}

export interface PermissionStatus {
  microphone: boolean;
  accessibility: boolean;
}

export interface ModelDescriptor {
  id: string;
  kind: "stt" | "llm";
  name: string;
  size_bytes: number;
  sha256: string | null;
  url: string;
  engine: string;
  ram_peak_mb: number;
  tier: string;
}

export interface ModelStatus {
  descriptor: ModelDescriptor;
  installed: boolean;
  bytes_on_disk: number;
  path: string | null;
}

export interface ModelProgress {
  id: string;
  bytes_downloaded: number;
  total_bytes: number;
  eta_secs: number | null;
}

export interface TierRecommendation {
  tier: string;
  stt_model_id: string;
  llm_model_id: string;
  physical_ram_gb: number;
  low_ram_warning: boolean;
}

export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export async function getPublicSettings(): Promise<Settings> {
  return invoke<Settings>("get_public_settings");
}

export async function setSettings(settings: Settings): Promise<void> {
  await invoke("set_settings", { settings });
}

export async function setDictationEnabled(enabled: boolean): Promise<void> {
  await invoke("set_dictation_enabled", { enabled });
}

export async function setHandsFreeEnabled(enabled: boolean): Promise<void> {
  await invoke("set_hands_free_enabled", { enabled });
}

export async function setPolishingEnabled(enabled: boolean): Promise<void> {
  await invoke("set_polishing_enabled", { enabled });
}

export async function setStreamingPillEnabled(enabled: boolean): Promise<void> {
  await invoke("set_streaming_pill_enabled", { enabled });
}

export async function setAutoPasteEnabled(enabled: boolean): Promise<void> {
  await invoke("set_auto_paste_enabled", { enabled });
}

/// Persist the "quiet other audio while dictating" mode. Dedicated
/// single-field setter (mirrors `setAutoPasteEnabled` etc.) so the tray
/// popover can change it — `set_settings` is gated to the settings /
/// onboarding webviews. Broadcasts `settings:changed`.
export async function setAudioAttenuation(
  value: "off" | "duck" | "pause",
): Promise<void> {
  await invoke("set_audio_attenuation", { value });
}

export async function setLiveTranscriptEnabled(enabled: boolean): Promise<void> {
  await invoke("set_live_transcript_enabled", { enabled });
}

export async function setEnableThinking(enabled: boolean): Promise<void> {
  await invoke("set_enable_thinking", { enabled });
}

/// Push measured content dimensions (logical px) up to the Rust side
/// so the pill window resizes to fit wrapped streaming text. Each
/// side is independently optional — pass only what changed. Called by
/// `Pill.tsx`'s ResizeObserver; Rust clamps each side before applying.
export async function pillResize(opts: {
  width?: number;
  height?: number;
}): Promise<void> {
  await invoke("pill_resize", { args: opts });
}

export async function setAppearance(
  appearance: "light" | "dark",
): Promise<void> {
  await invoke("set_appearance", { appearance });
}

export async function getRecentTranscripts(): Promise<RecentTranscript[]> {
  return invoke<RecentTranscript[]>("get_recent_transcripts");
}

export async function getTimeSavedStats(period: Period): Promise<TimeSavedStats> {
  return invoke<TimeSavedStats>("get_time_saved_stats", { period });
}

/// Whether the tray popover should surface the donation nudge right now,
/// and for which milestone. `null` = show nothing (opted out, snoozed,
/// or no new milestone crossed). The gating lives in the Rust `donation`
/// module so the frontend never re-implements it.
export async function getDonationPrompt(): Promise<DonationPrompt | null> {
  return invoke<DonationPrompt | null>("get_donation_prompt");
}

/// "Donate" / "I already donated" — stop future milestone nudges
/// permanently. The passive "Support Vox" row remains for repeat giving.
export async function donationOptOut(): Promise<void> {
  await invoke("donation_opt_out");
}

/// "Maybe later" — record the shown tier (so it never re-fires) and hold
/// further nudges for the snooze floor.
export async function donationSnooze(milestoneSecs: number): Promise<void> {
  await invoke("donation_snooze", { milestoneSecs });
}

export async function openSettingsWindow(): Promise<void> {
  await invoke("open_settings");
}

export async function closeTrayPopover(): Promise<void> {
  await invoke("close_tray_popover");
}

export async function quitApp(): Promise<void> {
  await invoke("quit_app");
}

export async function openAccessibilitySettings(): Promise<void> {
  await invoke("open_accessibility_settings");
}

export async function openMicrophoneSettings(): Promise<void> {
  await invoke("open_microphone_settings");
}

/// Enumerate the currently-connected audio input devices. The
/// dropdown in Settings → General → Microphone and the tray popover's
/// mic row both call this to populate their lists. `is_default`
/// flags whichever device macOS would pick if `audio_input_device`
/// is left as `null`/empty.
export async function listAudioInputDevices(): Promise<AudioInputDevice[]> {
  return invoke<AudioInputDevice[]>("list_audio_input_devices");
}

/// Persist the user's chosen audio input device. `null` (or empty)
/// means follow the macOS system default. Callable from any window
/// — `set_settings` is gated to the settings / onboarding webviews,
/// but this single-field setter is the way the tray popover's mic
/// picker actually persists. The backend also opens the device
/// briefly on a background thread so the iPhone Continuity
/// Microphone wake UI fires immediately on selection.
export async function setAudioInputDevice(name: string | null): Promise<void> {
  await invoke("set_audio_input_device", { device: name });
}

/// Open an http(s) URL in the user's default browser. Tauri webviews
/// ignore `target="_blank"`, so external links must round-trip through
/// this command.
export async function openExternalUrl(url: string): Promise<void> {
  await invoke("open_external_url", { url });
}

/// Wipe all Vox state (settings, modes, models, llama-server binaries,
/// TCC entries, autostart) and quit. The user must drag `Vox.app` to
/// Trash manually to finish uninstalling.
export async function uninstallApp(): Promise<void> {
  await invoke("uninstall_app");
}

export async function checkPermissions(): Promise<PermissionStatus> {
  return invoke<PermissionStatus>("check_permissions");
}

export async function requestMicrophone(): Promise<boolean> {
  return invoke<boolean>("request_microphone");
}

export async function replayOnboarding(): Promise<void> {
  await invoke("replay_onboarding");
}

export async function finishOnboarding(): Promise<void> {
  await invoke("finish_onboarding");
}

export async function getLaunchAtLogin(): Promise<boolean> {
  return invoke<boolean>("get_launch_at_login");
}

export async function setLaunchAtLogin(enabled: boolean): Promise<void> {
  await invoke("set_launch_at_login", { enabled });
}

export async function copyText(text: string): Promise<void> {
  await invoke("copy_text", { text });
}

export async function checkForUpdates(): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>("check_for_updates");
}

/// Manual "Check now" trigger from Settings → About. Bypasses the 6 h
/// cooldown by design — a user clicking "Check now" wants an answer
/// right now, not a stale cached result. The result is also re-emitted
/// on `updates:state` for every subscribed window.
export async function checkForUpdatesNow(): Promise<UpdateState> {
  return invoke<UpdateState>("check_for_updates_now");
}

/// Snapshot the in-process updater state. Used by windows that mount
/// fresh (Settings, tray popover) so the initial render doesn't have
/// to wait for the next `updates:state` emit. Live updates still flow
/// through `onUpdateState`.
export async function getUpdateState(): Promise<UpdateState> {
  return invoke<UpdateState>("get_update_state");
}

/// "Later" — per-version dismissal of the tray banner + dot.
/// Settings → About remains visible regardless. Resolves with the
/// resulting state for callers that want to update local UI
/// optimistically.
export async function dismissUpdate(): Promise<UpdateState> {
  return invoke<UpdateState>("dismiss_update");
}

/// Persist the periodic-check toggle. When `false`, the background
/// scheduler in `updater.rs` no-ops but the manual button still
/// works. Mirrors `setStreamingPillEnabled` etc.
export async function setAutoUpdateCheckEnabled(enabled: boolean): Promise<void> {
  await invoke("set_auto_update_check_enabled", { enabled });
}

/// Subscribe to the cross-window `updates:state` event. Fires on
/// every transition (checking → available / up-to-date / error) and
/// on every "Later" dismissal or successful install (the latter
/// rarely observed since the app restarts on install). Returns the
/// unsubscribe function.
export async function onUpdateState(
  cb: (s: UpdateState) => void,
): Promise<UnlistenFn> {
  return listen<UpdateState>("updates:state", (e) => cb(e.payload));
}

/// Kick off the actual download + install. Resolves only on failure —
/// success ends with `app.restart()` on the Rust side, so the webview
/// goes away with the process. Listen on `updater:progress` for the
/// running state via `onUpdaterProgress`.
export async function downloadAndInstallUpdate(): Promise<void> {
  await invoke("download_and_install_update");
}

export async function onUpdaterProgress(
  cb: (p: UpdaterProgress) => void,
): Promise<UnlistenFn> {
  return listen<UpdaterProgress>("updater:progress", (e) => cb(e.payload));
}

export const getAppVersion = (): Promise<string> => invoke("get_app_version");

export async function listModels(): Promise<ModelStatus[]> {
  return invoke<ModelStatus[]>("list_models");
}

export async function downloadModel(id: string): Promise<void> {
  await invoke("download_model", { id });
}

export async function cancelDownload(id: string): Promise<boolean> {
  return invoke<boolean>("cancel_download", { id });
}

export async function deleteModel(id: string): Promise<void> {
  await invoke("delete_model", { id });
}

export async function loadModelWarm(id: string): Promise<void> {
  await invoke("load_model_warm", { id });
}

export async function recommendTier(): Promise<TierRecommendation> {
  return invoke<TierRecommendation>("recommend_tier");
}

export async function acceptGemmaTos(): Promise<void> {
  await invoke("accept_gemma_tos");
}

export async function onModelProgress(
  cb: (p: ModelProgress) => void,
): Promise<UnlistenFn> {
  return listen<ModelProgress>("model:progress", (e) => cb(e.payload));
}

export async function onModelDone(
  cb: (p: { id: string; already_installed?: boolean }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string; already_installed?: boolean }>(
    "model:done",
    (e) => cb(e.payload),
  );
}

export async function onModelError(
  cb: (p: { id: string; error: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string; error: string }>("model:error", (e) =>
    cb(e.payload),
  );
}

export async function onModelCanceled(
  cb: (p: { id: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string }>("model:canceled", (e) => cb(e.payload));
}

export interface InferenceServerStatus {
  id: string;
  version: string;
  installed: boolean;
  /** False on platforms where we don't ship a llama.cpp binary. The
   *  frontend hides the polish path entirely when this is false. */
  supported_host: boolean;
  binary_path: string | null;
  size_bytes: number;
}

export async function downloadInferenceServer(): Promise<void> {
  await invoke("download_inference_server");
}

export async function cancelInferenceServerDownload(): Promise<boolean> {
  return invoke<boolean>("cancel_inference_server_download");
}

export async function inferenceServerStatus(): Promise<InferenceServerStatus> {
  return invoke<InferenceServerStatus>("inference_server_status");
}

export async function restartInferenceServer(): Promise<void> {
  await invoke("restart_inference_server");
}

export async function deleteInferenceServer(): Promise<void> {
  await invoke("delete_inference_server");
}

export type AppleFmAvailabilityCode =
  | "available"
  | "device_not_eligible"
  | "intelligence_not_enabled"
  | "model_not_ready"
  | "os_too_old"
  | "internal_error"
  | "framework_missing";

export interface AppleFmAvailability {
  available: boolean;
  code: AppleFmAvailabilityCode;
  message: string;
  /** False on non-macOS / when the apple-fm feature was compiled out.
   *  Frontend hides the entire Apple Intelligence section when false. */
  supported_host: boolean;
}

export async function appleFmAvailability(): Promise<AppleFmAvailability> {
  return invoke<AppleFmAvailability>("apple_fm_availability");
}

// Mirror of models.rs::PARAKEET_SUPPORTED_LANGUAGES — keep in sync.
// Source: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
export const PARAKEET_SUPPORTED_LANGUAGES: readonly string[] = [
  "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el",
  "hu", "it", "lv", "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es",
  "sv", "ru", "uk",
] as const;

export function parakeetSupportsLanguage(code: string): boolean {
  return PARAKEET_SUPPORTED_LANGUAGES.includes(code);
}

// Pick the recommended STT engine id given language preference and
// runtime availability. Mirrors models.rs::recommended_stt_engine_for.
export function recommendedSttEngineFor(
  language: string,
  parakeetAvailable: boolean,
): "parakeet-local" | "whisper-local" {
  if (!parakeetAvailable) return "whisper-local";
  if (language === "auto" || parakeetSupportsLanguage(language)) {
    return "parakeet-local";
  }
  return "whisper-local";
}

// Render an ISO 639-1 code as a UI-facing display name. Falls back to
// the upper-cased code when Intl.DisplayNames doesn't recognise it
// (older WebKit, exotic codes) so we never show a blank label.
export function languageDisplayName(code: string): string {
  if (code === "auto") return "Detect automatically";
  try {
    return new Intl.DisplayNames(["en"], { type: "language" }).of(code) ?? code.toUpperCase();
  } catch {
    return code.toUpperCase();
  }
}

// Mirror of commands::parakeet_availability shape.
export interface ParakeetAvailability {
  available: boolean;
  reason: string | null;
}

// Tolerates the backend command not being registered yet (parallel
// implementation work) — returns `{ available: false, reason: ... }` so
// the UI degrades to Whisper-only without throwing.
export async function parakeetAvailability(): Promise<ParakeetAvailability> {
  try {
    return await invoke<ParakeetAvailability>("parakeet_availability");
  } catch (e) {
    return { available: false, reason: String(e) };
  }
}

export async function onBinaryProgress(
  cb: (p: ModelProgress) => void,
): Promise<UnlistenFn> {
  return listen<ModelProgress>("binary:progress", (e) => cb(e.payload));
}

export async function onBinaryDone(
  cb: (p: { id: string; already_installed?: boolean }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string; already_installed?: boolean }>(
    "binary:done",
    (e) => cb(e.payload),
  );
}

export async function onBinaryError(
  cb: (p: { id: string; error: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string; error: string }>("binary:error", (e) =>
    cb(e.payload),
  );
}

export async function onBinaryCanceled(
  cb: (p: { id: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string }>("binary:canceled", (e) => cb(e.payload));
}

export async function onSettingsChanged(
  cb: (s: Settings) => void,
): Promise<UnlistenFn> {
  return listen<Settings>("settings:changed", (e) => cb(e.payload));
}

/// Fired by the backend whenever the Voice Mode list changes (save,
/// delete, duplicate, import, reset). Payload is the full current
/// list. Subscribed by every window that displays a mode picker so
/// custom modes show up without a Vox restart.
export async function onModesChanged(
  cb: (modes: VoiceMode[]) => void,
): Promise<UnlistenFn> {
  return listen<VoiceMode[]>("modes:changed", (e) => cb(e.payload));
}

export async function onTranscriptsChanged(
  cb: (rec: RecentTranscript[]) => void,
): Promise<UnlistenFn> {
  return listen<RecentTranscript[]>("transcripts:changed", (e) => cb(e.payload));
}

export async function onStatsChanged(
  handler: () => void,
): Promise<UnlistenFn> {
  return listen<unknown>("stats:changed", () => handler());
}

export async function cancelSession(sessionId: number): Promise<void> {
  await invoke("cancel_session", { sessionId });
}

export interface RePolishResult {
  text: string;
  /**
   * True when the LLM step failed (or returned empty) and the new
   * text is the unpolished raw transcript shaped only by the mode's
   * post-processing. Callers should surface a "Raw copy" hint.
   */
  warning: boolean;
  mode_id: string;
  mode_name: string;
}

/**
 * Re-run the LLM polish on a Recent transcript using a different
 * Voice Mode. The result is copied to the clipboard and the matching
 * `CompletedEntry` in the ring buffer is updated in place — every
 * window subscribed to `sessions:changed` sees the new text.
 *
 * Fails with a string error when the entry is no longer in the
 * ring (rotated out by newer captures), when the mode id is unknown,
 * or when the clipboard write fails. LLM-step failures do NOT throw —
 * they surface as `warning: true` in the response.
 */
export async function rePolishTranscript(
  sessionId: number,
  modeId: string,
): Promise<RePolishResult> {
  return invoke<RePolishResult>("re_polish_transcript", {
    sessionId,
    modeId,
  });
}

export async function sessionsSnapshot(): Promise<SessionsSnapshot> {
  return invoke<SessionsSnapshot>("sessions_snapshot");
}

export async function onSessionsChanged(
  cb: (p: SessionsSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<SessionsSnapshot>("sessions:changed", (e) => cb(e.payload));
}

export async function onPillToast(
  cb: (p: { message: string; ms: number }) => void,
): Promise<UnlistenFn> {
  return listen<{ message: string; ms: number }>("pill:toast", (e) =>
    cb(e.payload),
  );
}

/**
 * Cumulative streaming text — `text` is the full transcript (or polish)
 * so far, NOT a delta. The pill rate-limits visible chars/sec; `final`
 * tells the UI to flush instantly.
 */
export interface StreamPayload {
  session_id: number;
  text: string;
  final: boolean;
}

export async function onPillTranscriptStream(
  cb: (p: StreamPayload) => void,
): Promise<UnlistenFn> {
  return listen<StreamPayload>("pill:transcript_stream", (e) => cb(e.payload));
}

export async function onPillPolishStream(
  cb: (p: StreamPayload) => void,
): Promise<UnlistenFn> {
  return listen<StreamPayload>("pill:polish_stream", (e) => cb(e.payload));
}

export async function listModes(): Promise<VoiceMode[]> {
  return invoke<VoiceMode[]>("list_modes");
}

export async function getMode(id: string): Promise<VoiceMode | null> {
  return invoke<VoiceMode | null>("get_mode", { id });
}

export async function saveMode(mode: VoiceMode): Promise<void> {
  await invoke("save_mode", { mode });
}

export async function deleteMode(id: string): Promise<void> {
  await invoke("delete_mode", { id });
}

export async function duplicateMode(id: string): Promise<VoiceMode> {
  return invoke<VoiceMode>("duplicate_mode", { id });
}

export async function setActiveMode(id: string): Promise<VoiceMode> {
  return invoke<VoiceMode>("set_active_mode", { id });
}

export async function exportMode(id: string): Promise<string> {
  return invoke<string>("export_mode", { id });
}

export async function importModeJson(json: string): Promise<VoiceMode> {
  return invoke<VoiceMode>("import_mode_json", { json });
}

export async function resetDefaultModes(): Promise<number> {
  return invoke<number>("reset_default_modes");
}

export async function cycleActiveMode(): Promise<VoiceMode> {
  return invoke<VoiceMode>("cycle_active_mode");
}

export async function suggestActiveMode(): Promise<string | null> {
  return invoke<string | null>("suggest_active_mode");
}

export interface ActiveHotkey {
  configured: string;
  bound: string;
  fell_back: boolean;
  /** "shortcut" | "chord" | "". "chord" = CGEventTap modifier-only
   *  listener; "shortcut" = `tauri-plugin-global-shortcut` (Carbon). */
  mechanism: string;
  error: string | null;
  error_chain: string[];
}

export interface DevBinaryInfo {
  binary_path: string;
  bundle_identifier: string;
  is_dev_build: boolean;
  accessibility_granted: boolean;
}

export async function revealDevBinaryPath(): Promise<DevBinaryInfo> {
  return invoke<DevBinaryInfo>("reveal_dev_binary_path");
}

export interface HotkeyDiagnostic {
  purpose: string;
  configured: string;
  bound: string;
  fell_back: boolean;
  /** "shortcut" | "chord" | "". "chord" = CGEventTap modifier-only
   *  listener; "shortcut" = `tauri-plugin-global-shortcut` (Carbon). */
  mechanism: string;
  error: string | null;
  error_chain: string[];
}

export interface DiagnoseHotkeys {
  entries: HotkeyDiagnostic[];
}

export async function getActiveHotkey(): Promise<ActiveHotkey> {
  return invoke<ActiveHotkey>("get_active_hotkey");
}

export async function diagnoseHotkeys(): Promise<DiagnoseHotkeys> {
  return invoke<DiagnoseHotkeys>("diagnose_hotkeys");
}

export interface ActiveModePayload {
  id: string;
  name: string;
  icon: string;
}

export interface ModeSuggestionPayload {
  id: string;
  name: string;
  icon: string;
}

export async function onPillActiveMode(
  cb: (p: ActiveModePayload) => void,
): Promise<UnlistenFn> {
  return listen<ActiveModePayload>("pill:active_mode", (e) => cb(e.payload));
}

export async function onPillModeSuggestion(
  cb: (p: ModeSuggestionPayload | null) => void,
): Promise<UnlistenFn> {
  return listen<ModeSuggestionPayload | null>("pill:mode_suggestion", (e) =>
    cb(e.payload),
  );
}
