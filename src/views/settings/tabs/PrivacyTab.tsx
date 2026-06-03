import { For } from "solid-js";
import { type Settings } from "@/lib/ipc";
import { deviceWord, isMac } from "@/lib/platform";

export function PrivacyTab(_props: { s: Settings }) {
  const guarantees: { title: string; detail: string }[] = [
    {
      title: `Audio never leaves the ${deviceWord}`,
      detail:
        "Captured into memory, transcribed locally, freed before the pipeline ends.",
    },
    {
      title: "Transcripts stay on the device",
      detail: isMac
        ? "Whisper, Parakeet, and Gemma all run on Apple Silicon. No network calls in the dictation path."
        : "Whisper, Parakeet, and Gemma all run locally on your PC. No network calls in the dictation path.",
    },
    {
      title: "Nothing is written to disk",
      detail:
        "Audio is never saved. Transcripts live only in a process-lifetime ring buffer.",
    },
    {
      title: "No telemetry",
      detail: "No analytics, no crash uploads, no remote logs.",
    },
    {
      title: "No keys to leak",
      detail:
        "There are no API keys to store, because there are no third-party services.",
    },
    {
      // Only network access outside model + binary downloads. Worth
      // disclosing because it's a recurring background call — the
      // toggle for it lives in Settings → General.
      title: "Update check",
      detail:
        "Vox checks GitHub once a day for new releases. No identifiers are sent.",
    },
  ];

  return (
    <>
      <div
        class="liquid-card mb-5 overflow-hidden rounded-[14px] ring-1 ring-[var(--color-border)]"
      >
        <div class="flex items-start gap-3 border-b border-[var(--color-border)] px-4 py-4">
          <div
            class="flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px]"
            style={{
              background: "var(--color-accent)",
              "box-shadow":
                "inset 0 1px 0 rgba(255,255,255,0.35), 0 1px 2px rgba(0,0,0,0.15)",
            }}
            aria-hidden="true"
          >
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="white"
              stroke-width="2.25"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <rect x="4" y="11" width="16" height="10" rx="2" />
              <path d="M8 11V7a4 4 0 0 1 8 0v4" />
            </svg>
          </div>
          <div class="flex flex-col">
            <h3 class="text-[14px] font-semibold text-[var(--color-ink)]">
              Fully on-device
            </h3>
            <p class="mt-0.5 text-[12px] text-[var(--color-ink-mute)]">
              Vox runs entirely on this {deviceWord}. There is no setting that
              would send your audio or transcripts to any external
              service.
            </p>
          </div>
        </div>
        <ul class="flex flex-col">
          <For each={guarantees}>
            {(g, i) => (
              <li
                class="flex items-start gap-3 px-4 py-3"
                classList={{
                  "border-b border-[var(--color-border)]":
                    i() < guarantees.length - 1,
                }}
              >
                <svg
                  class="mt-0.5 shrink-0 text-[var(--color-success)]"
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"
                >
                  <polyline points="5 12 10 17 19 7" />
                </svg>
                <div class="flex flex-col">
                  <span class="text-[13px] text-[var(--color-ink)]">
                    {g.title}
                  </span>
                  <span class="text-[11px] text-[var(--color-ink-mute)]">
                    {g.detail}
                  </span>
                </div>
              </li>
            )}
          </For>
        </ul>
      </div>
    </>
  );
}
