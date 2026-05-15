import {
  createSignal,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import {
  getLaunchAtLogin,
  getSettings,
  onSettingsChanged,
  setSettings,
  type Settings,
} from "../lib/ipc";
import { Header } from "./settings/components/Header";
import { Loader } from "./settings/components/Loader";
import { Sidebar } from "./settings/components/Sidebar";
import { type Tab } from "./settings/constants";
import { AboutTab } from "./settings/tabs/AboutTab";
import { DictionaryTab } from "./settings/tabs/DictionaryTab";
import { GeneralTab } from "./settings/tabs/GeneralTab";
import { HotkeysTab } from "./settings/tabs/HotkeysTab";
import { ModelsTab } from "./settings/tabs/ModelsTab";
import { ModesTab } from "./settings/tabs/ModesTab";
import { PrivacyTab } from "./settings/tabs/PrivacyTab";

export default function SettingsView() {
  const [tab, setTab] = createSignal<Tab>("general");
  const [loaded, setLoaded] = createSignal(false);
  const [state, setState] = createSignal<Settings | null>(null);
  const [savedFlash, setSavedFlash] = createSignal(false);

  let unlistenSettings: (() => void) | null = null;
  // Bumped on local saves so the settings:changed echo can be ignored —
  // a redundant setState would force every controlled input to re-render
  // mid-typing.
  let lastSelfSaveAt = 0;

  onMount(async () => {
    try {
      const s = await getSettings();
      try {
        const osLaunch = await getLaunchAtLogin();
        if (osLaunch !== s.launch_at_login) {
          s.launch_at_login = osLaunch;
        }
      } catch (e) {
        console.warn("getLaunchAtLogin failed:", e);
      }
      setState(s);
    } finally {
      setLoaded(true);
    }

    unlistenSettings = await onSettingsChanged((next) => {
      if (Date.now() - lastSelfSaveAt < 500) return;
      setState(next);
    });
  });

  onCleanup(() => {
    unlistenSettings?.();
  });

  async function update(patch: Partial<Settings>) {
    const cur = state();
    if (!cur) return;
    const next = { ...cur, ...patch } as Settings;
    setState(next);
    try {
      lastSelfSaveAt = Date.now();
      await setSettings(next);
      setSavedFlash(true);
      setTimeout(() => setSavedFlash(false), 1200);
    } catch (err) {
      console.error("Save failed:", err);
    }
  }

  return (
    <div class="liquid-canvas flex h-screen w-screen text-[var(--color-ink)]">
      <Show when={loaded() && state()} fallback={<Loader />}>
        {(s) => (
          <>
            <Sidebar tab={tab()} onTab={setTab} />
            <main class="flex-1 overflow-y-auto scrollbar-thin px-8 py-6">
              <Header tab={tab()} savedFlash={savedFlash()} />
              <Show when={tab() === "general"}>
                <GeneralTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "hotkeys"}>
                <HotkeysTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "models"}>
                <ModelsTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "modes"}>
                <ModesTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "dictionary"}>
                <DictionaryTab s={s()} update={update} />
              </Show>
              <Show when={tab() === "privacy"}>
                <PrivacyTab s={s()} />
              </Show>
              <Show when={tab() === "about"}>
                <AboutTab />
              </Show>
            </main>
          </>
        )}
      </Show>
    </div>
  );
}
