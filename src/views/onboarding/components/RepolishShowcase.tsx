import { SparkleGlyph } from "@/components/SparkleGlyph";

export function RepolishShowcase() {
  return (
    <div class="liquid-card flex flex-col gap-3 rounded-[10px] bg-[var(--color-surface-raised)] px-4 py-4 ring-1 ring-[var(--color-border)]">
      <RepolishMini />
      <div class="flex flex-col gap-1">
        <h3 class="text-[14px] font-semibold leading-tight text-[var(--color-ink)]">
          Wrong mode? Repolish it.
        </h3>
        <p class="text-[12px] leading-[17px] text-[var(--color-ink-mute)]">
          Dictated a casual note but realized it should've been an email? Open the Vox menu-bar icon, hover any recent transcript, and tap the sparkle to re-clean it with a different Voice Mode — the new version lands on your clipboard instantly.
        </p>
      </div>
    </div>
  );
}

/**
 * Mini popover + connector + re-polish picker. Mirrors the real tray
 * popover's Recent row and "Re-polish as…" menu so the user recognises
 * both surfaces when they actually open the menu bar icon. The dashed
 * arrow visually links the sparkle button to the mode picker that pops
 * out of it.
 */
function RepolishMini() {
  return (
    <div class="relative h-[120px] w-full" aria-hidden="true">
      <div
        class="liquid-glass rounded-[14px] px-2.5 py-2 flex flex-col gap-1"
        style={{
          position: "absolute",
          left: "8px",
          top: "12px",
          width: "240px",
          height: "96px",
        }}
      >
        <div class="px-1.5 pt-0.5 pb-1 text-[9px] font-semibold uppercase tracking-[0.06em] text-[var(--color-ink-faint)]">
          Recent
        </div>
        <div class="flex items-center gap-2 rounded-[8px] bg-[var(--color-overlay)] px-2 py-1.5">
          <span class="w-8 shrink-0 text-[9px] font-medium text-[var(--color-ink-faint)]">
            2m
          </span>
          <span class="flex-1 truncate text-[10px] text-[var(--color-ink)]">
            hey just checking in on the deck
          </span>
          <span class="inline-flex h-5 w-5 items-center justify-center rounded bg-[var(--color-accent-soft)] text-[var(--color-accent)] onboarding-sparkle-pulse">
            <SparkleGlyph width={12} height={12} />
          </span>
        </div>
      </div>
      <svg
        style={{
          position: "absolute",
          top: "44px",
          left: "200px",
          width: "80px",
          height: "30px",
          "pointer-events": "none",
          overflow: "visible",
        }}
      >
        <path
          d="M0 12 C 24 12, 36 4, 64 4"
          stroke="var(--color-ink-faint)"
          stroke-width="1"
          stroke-dasharray="2 3"
          fill="none"
        />
        <polyline
          points="58 -1 64 4 58 9"
          fill="none"
          stroke="var(--color-ink-faint)"
          stroke-width="1"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
      <div
        class="liquid-glass rounded-[10px] py-1 flex flex-col"
        style={{
          position: "absolute",
          right: "6px",
          top: "6px",
          width: "148px",
        }}
      >
        <div class="px-2 pb-0.5 pt-1 text-[8px] font-semibold uppercase tracking-[0.06em] text-[var(--color-ink-faint)]">
          Re-polish as…
        </div>
        <div class="flex items-center gap-1.5 px-2 py-1 text-[10px] bg-[var(--color-accent-soft)] text-[var(--color-accent)]">
          <span aria-hidden="true">📧</span>
          <span>Email</span>
        </div>
        <div class="flex items-center gap-1.5 px-2 py-1 text-[10px] text-[var(--color-ink)]">
          <span aria-hidden="true">💭</span>
          <span>Chat</span>
        </div>
        <div class="flex items-center gap-1.5 px-2 py-1 text-[10px] text-[var(--color-ink)]">
          <span aria-hidden="true">📝</span>
          <span>Notes</span>
        </div>
      </div>
    </div>
  );
}
