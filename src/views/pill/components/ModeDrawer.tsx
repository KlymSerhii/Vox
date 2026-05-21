import { Show } from "solid-js";
import { type ActiveModePayload } from "../types";

/// Mode drawer: an "attached half-pill" that grows downward from the bottom
/// of the main listening pill when the mode-cycle hotkey fires during a
/// recording session. No gap between pill and drawer — they share a continuous
/// flat seam. The main pill's bottom border-radius transitions to 0 in lockstep.
///
/// Open:  height 0→25px + opacity 0→1 over 220ms ease-out.
/// Close: fading=true drives opacity 1→0 over 220ms, then the parent
///        clears `mode` (unmounting the content) after the same 220ms timer.
///
/// `mode` null means nothing is mounted — the drawer collapses to 0 height.
export function ModeDrawer(props: { mode: ActiveModePayload | null; fading: boolean }) {
  return (
    <div
      class="pointer-events-none overflow-hidden"
      style={{
        // Grow downward from height 0. The 25px target matches the
        // "half-pill" spec: padding (3px top/bottom) + content (~11px)
        // + border-bottom (1px) = ~25px. Use height transition (not
        // scaleY) so the rounded bottom corners aren't distorted.
        height: props.mode ? "25px" : "0px",
        opacity: props.mode && !props.fading ? 1 : 0,
        transition: "height 220ms ease-out, opacity 220ms ease-out",
        // Full width of the wrapper so longer mode names (e.g. "Code Comment")
        // aren't clipped; visual distinction from the main pill comes from the
        // height difference and the rounded-bottom-only shape.
        width: "100%",
        // Flat top edge (shared seam with pill), rounded bottom corners.
        // 18px mirrors the effective radius on the main pill's top corners
        // (border-radius:9999px on 116×36px → min(58,18) = 18px).
        "border-radius": "0 0 18px 18px",
        background: "var(--color-pill-surface)",
        "backdrop-filter": "var(--glass-blur)",
        "-webkit-backdrop-filter": "var(--glass-blur)",
        // No top border — the shared seam must be invisible.
        "border-left": "1px solid var(--glass-rim)",
        "border-right": "1px solid var(--glass-rim)",
        "border-bottom": "1px solid var(--glass-rim)",
      }}
      aria-hidden="true"
    >
      <Show when={props.mode}>
        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "center",
            gap: "5px",
            padding: "3px 8px",
            "font-size": "10.5px",
            "font-weight": 500,
            color: "var(--color-ink-mute)",
            "white-space": "nowrap",
          }}
        >
          <span>{props.mode!.icon}</span>
          <span>{props.mode!.name}</span>
        </div>
      </Show>
    </div>
  );
}
