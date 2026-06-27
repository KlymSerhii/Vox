import { HotkeyKeys } from "@/components/HotkeyKeys";

export function VoiceModesShowcase() {
  return (
    <div class="liquid-card flex flex-col gap-3 rounded-[10px] bg-[var(--color-surface-raised)] px-4 py-4 ring-1 ring-[var(--color-border)]">
      <VoiceModesFan />
      <div class="flex flex-col gap-1">
        <h3 class="text-[14px] font-semibold leading-tight text-[var(--color-ink)]">
          Match your tone to the moment.
        </h3>
        <p class="text-[12px] leading-[17px] text-[var(--color-ink-mute)]">
          Seven built-in modes — Email, Chat, Notes, Code comment, Summary, and more — change how your transcript gets cleaned up.
        </p>
      </div>
      <div class="flex flex-wrap items-center gap-1 text-[12px] text-[var(--color-ink-mute)]">
        <span>Cycle anytime with</span>
        <HotkeyKeys spec="Alt+M" />
      </div>
    </div>
  );
}

/**
 * Three angled liquid-glass chips that preview how the same dictation
 * reads in three different voice modes. Center chip leads (no rotation,
 * soft accent ring) so the eye lands there first; the side chips fan
 * out at ±16° to suggest "there are more". Emoji glyphs are mirrored
 * from `modes.rs::builtins()` so this card and the tray popover's
 * mode picker show the same icon per mode.
 */
function VoiceModesFan() {
  const chip = (
    body: string,
    style: { transform: string; "z-index": number; center?: boolean },
    emoji: string,
    label: string,
  ) => (
    <div
      class="liquid-glass absolute flex flex-col gap-1 px-2.5 py-2"
      classList={{
        "ring-1 ring-[var(--color-accent-soft)]": !!style.center,
      }}
      style={{
        width: "170px",
        height: "96px",
        "border-radius": "10px",
        "box-shadow": "var(--glass-shadow-stack-sm)",
        left: "50%",
        top: "12px",
        transform: style.transform,
        "z-index": style["z-index"],
      }}
    >
      <div class="flex items-center gap-1.5">
        <span
          aria-hidden="true"
          style={{ "font-size": "14px", "line-height": "14px" }}
        >
          {emoji}
        </span>
        <span class="text-[10px] font-medium text-[var(--color-ink-mute)]">
          {label}
        </span>
      </div>
      <p class="text-[10px] leading-[14px] text-[var(--color-ink)]">
        {body}
      </p>
    </div>
  );
  return (
    <div class="relative h-[120px] w-full" aria-hidden="true">
      {chip(
        "Hi — just wanted to check in about the deck. Thanks!",
        {
          transform:
            "translateX(calc(-50% - 78px)) translateY(8px) rotate(-16deg)",
          "z-index": 10,
        },
        "📧",
        "Email",
      )}
      {chip(
        "• Check in on deck status",
        {
          transform: "translateX(calc(-50% + 78px)) translateY(8px) rotate(16deg)",
          "z-index": 10,
        },
        "📝",
        "Notes",
      )}
      {chip(
        "hey just checking in on the deck",
        {
          transform: "translateX(-50%) translateY(-2px) rotate(0deg)",
          "z-index": 30,
          center: true,
        },
        "💭",
        "Chat",
      )}
    </div>
  );
}
