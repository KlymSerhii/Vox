import { Show } from "solid-js";

/// Small violet circle anchored to the right edge of the pill. Shows
/// a count when 2 or more background sessions are running; just a
/// dot otherwise. Sits on top of the pill content via absolute
/// positioning so it never reflows the foreground text.
export function QueueDot(props: { count: number }) {
  return (
    <div
      class="pointer-events-none absolute"
      style={{
        right: "-3px",
        top: "50%",
        transform: "translateY(-50%)",
        width: "9px",
        height: "9px",
        "border-radius": "9999px",
        background: "var(--color-violet, #a78bfa)",
        // Halo ring matches the surrounding bg, not the translucent pill
        // surface — using --color-pill-surface in light mode produced a
        // visible cream halo around the violet dot that read as a paint
        // bug. --color-bg merges into the ambient backdrop at any theme.
        "box-shadow": "0 0 0 1.5px var(--color-bg), 0 0 8px rgba(167, 139, 250, 0.55)",
        display: "flex",
        "align-items": "center",
        "justify-content": "center",
      }}
      aria-hidden="true"
      title={`${props.count} background session${props.count === 1 ? "" : "s"}`}
    >
      <Show when={props.count >= 2}>
        <span
          style={{
            "font-size": "7px",
            "font-weight": "700",
            color: "white",
            "line-height": "1",
          }}
        >
          {props.count}
        </span>
      </Show>
    </div>
  );
}
