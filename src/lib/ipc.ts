import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Settings {
  hotkey: string;
  cancel_hotkey: string;
  launch_at_login: boolean;
  dictation_enabled: boolean;
  hands_free_enabled: boolean;
  language: string;
  onboarded: boolean;
  dictionary: string[];
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
  /// Show the transcript / polish text streaming inside the pill after
  /// the hotkey is released. Default true; tray popover + Settings →
  /// General both flip it.
  streaming_pill_enabled: boolean;
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
  auto_triggers: AutoTrigger[];
  built_in: boolean;
  created_at: number;
  updated_at: number;
}

export interface UpdateCheckResult {
  status: "available" | "up-to-date" | "error" | "unconfigured";
  current_version?: string;
  available_version?: string;
  message?: string;
}

export interface RecentTranscript {
  ts: number;
  text: string;
}

export type Period = "last_session" | "day" | "week" | "month" | "year";

export interface TimeSavedStats {
  saved_seconds: number;
  sessions: number;
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
