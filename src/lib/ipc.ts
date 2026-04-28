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
  // Local-only engines (cloud paths removed 2026-04-26).
  stt_engine: string;        // "whisper-local"
  stt_model_id: string;
  llm_engine: string;        // "gemma-local" | "none"
  llm_model_id: string;
  /// User-facing on/off toggle for transcript polishing. Survives across
  /// engine outages so the user's intent isn't lost when the local LLM
  /// is temporarily unavailable.
  polishing_enabled: boolean;
  gemma_tos_accepted: boolean;
  // Voice Modes.
  active_mode_id: string;
  mode_cycle_hotkey: string;
  auto_suggest_modes: boolean;
  // Appearance.
  appearance: "light" | "dark";
  // Out-of-process Gemma — see `inference_proc.rs`. Token used to
  // identify our `llama-server` children for orphan reaping. Not a
  // secret in the security sense (server is loopback-only).
  inference_api_key: string;
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

export async function setAppearance(
  appearance: "light" | "dark",
): Promise<void> {
  await invoke("set_appearance", { appearance });
}

export async function getRecentTranscripts(): Promise<RecentTranscript[]> {
  return invoke<RecentTranscript[]>("get_recent_transcripts");
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

// ---- Models ----

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

// ---- Inference server (out-of-process llama-server) ----
//
// Mirrors the model:* event family on the wire so the existing
// progress-bar component can be reused 1:1 in onboarding. The
// `id` field on every event matches `InferenceServerStatus.id`.

export interface InferenceServerStatus {
  id: string;
  version: string;
  installed: boolean;
  /** False on platforms where we don't ship a llama.cpp binary (e.g.
   *  Linux today). The frontend should hide the polish path entirely
   *  when this is false. */
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

/// Subscribe to backend `settings:changed` events. Fired after every
/// successful `set_settings` call, with the full new `Settings` payload.
/// Use this in any window that displays settings-derived UI (hotkey
/// glyphs, "polish transcripts" status, etc.) so they refresh
/// immediately when the user saves changes from the Settings window.
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

// ---- Voice Modes ----

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

// ---- Hotkey diagnostics ----

export interface ActiveHotkey {
  configured: string;
  bound: string;
  fell_back: boolean;
  /** "shortcut" | "chord" | "" — which mechanism the binding uses.
   *  "chord" means a CGEventTap modifier-only listener; "shortcut" is
   *  the standard `tauri-plugin-global-shortcut` (Carbon) path. */
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
  /** "shortcut" | "chord" | "" — which mechanism the binding uses.
   *  "chord" means a CGEventTap modifier-only listener; "shortcut" is
   *  the standard `tauri-plugin-global-shortcut` (Carbon) path. */
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
