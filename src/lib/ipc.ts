import { invoke } from "@tauri-apps/api/core";

export interface Settings {
  groq_api_key: string;
  anthropic_api_key: string;
  hotkey: string;
}

export interface TestConnectionResult {
  ok: boolean;
  message: string;
}

export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export async function setSettings(settings: Settings): Promise<void> {
  await invoke("set_settings", { settings });
}

export async function testGroqConnection(
  apiKey: string,
): Promise<TestConnectionResult> {
  return invoke<TestConnectionResult>("test_groq_connection", { apiKey });
}
