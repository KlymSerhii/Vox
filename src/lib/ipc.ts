import { invoke } from "@tauri-apps/api/core";

export interface Settings {
  groq_api_key: string;
  anthropic_api_key: string;
  hotkey: string;
  cancel_hotkey: string;
  cleanup_enabled: boolean;
  launch_at_login: boolean;
  play_sound: boolean;
  send_audio_to_cloud: boolean;
  dictation_enabled: boolean;
  onboarded: boolean;
  dictionary: string[];
}

export interface TestConnectionResult {
  ok: boolean;
  message: string;
}

export interface RecentTranscript {
  ts: number;
  text: string;
}

export interface PermissionStatus {
  microphone: boolean;
  accessibility: boolean;
}

export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

/**
 * Settings WITHOUT API keys. Safe to call from any window. Use this from
 * the pill, tray popover, or any non-secret context.
 */
export async function getPublicSettings(): Promise<Settings> {
  return invoke<Settings>("get_public_settings");
}

export async function setSettings(settings: Settings): Promise<void> {
  await invoke("set_settings", { settings });
}

export async function setDictationEnabled(enabled: boolean): Promise<void> {
  await invoke("set_dictation_enabled", { enabled });
}

export async function testGroqConnection(
  apiKey: string,
): Promise<TestConnectionResult> {
  return invoke<TestConnectionResult>("test_groq_connection", { apiKey });
}

export async function testAnthropicConnection(
  apiKey: string,
): Promise<TestConnectionResult> {
  return invoke<TestConnectionResult>("test_anthropic_connection", { apiKey });
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
