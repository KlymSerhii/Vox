/// Persistent "Polishing…" label row. Sits in normal flow above the
/// streaming body when the parent pill is in column layout (gated on
/// `state() === "polishing"`). Visible for the whole polishing stage;
/// crossfades in/out via CSS opacity transition.
export function PolishingLabelRow(props: { visible: boolean }) {
  return (
    <div
      class="polishing-label-row pointer-events-none"
      style={{
        display: "flex",
        "align-items": "center",
        gap: "6px",
        // 4 px gap to the streaming body underneath, per design spec.
        "margin-bottom": "4px",
        opacity: props.visible ? 1 : 0,
        transition: "opacity 180ms ease-out",
      }}
      aria-hidden="true"
    >
      <span
        class="polishing-dot"
        style={{
          width: "6px",
          height: "6px",
          "border-radius": "9999px",
          background: "var(--color-violet)",
          // Theme-aware halo via the --polishing-dot-glow token so the
          // light/dark themes each get a hue + alpha tuned for their
          // pill surface — replaces the previously hardcoded rgba.
          "box-shadow": "0 0 8px var(--polishing-dot-glow)",
          display: "inline-block",
        }}
      />
      <span
        style={{
          "font-size": "10.5px",
          "font-weight": 600,
          "letter-spacing": "0.04em",
          color: "var(--color-violet)",
          opacity: 0.85,
          "line-height": "1",
        }}
      >
        Polishing…
      </span>
    </div>
  );
}

/// Sticky header for the listening stage when the live-transcript opt-in
/// is on. Mirrors PolishingLabelRow / TranscribingLabelRow with coral
/// instead of violet/cyan — the pulsing dot doubles as the recording
/// indicator so the user can tell at a glance that the mic is hot even
/// when nobody is speaking yet.
export function ListeningLabelRow() {
  return (
    <div
      class="polishing-label-row pointer-events-none"
      style={{
        display: "flex",
        "align-items": "center",
        gap: "6px",
        "margin-bottom": "4px",
        opacity: 1,
        transition: "opacity 180ms ease-out",
      }}
      aria-hidden="true"
    >
      <span
        class="polishing-dot"
        style={{
          width: "6px",
          height: "6px",
          "border-radius": "9999px",
          background: "var(--color-coral)",
          "box-shadow": "0 0 8px var(--color-coral)",
          display: "inline-block",
        }}
      />
      <span
        style={{
          "font-size": "10.5px",
          "font-weight": 600,
          "letter-spacing": "0.04em",
          color: "var(--color-coral)",
          opacity: 0.85,
          "line-height": "1",
        }}
      >
        Listening…
      </span>
    </div>
  );
}

/// Sticky header for the transcribing stage — mirrors PolishingLabelRow
/// with cyan instead of violet.
export function TranscribingLabelRow() {
  return (
    <div
      class="polishing-label-row pointer-events-none"
      style={{
        display: "flex",
        "align-items": "center",
        gap: "6px",
        "margin-bottom": "4px",
        opacity: 1,
        transition: "opacity 180ms ease-out",
      }}
      aria-hidden="true"
    >
      <span
        class="polishing-dot"
        style={{
          width: "6px",
          height: "6px",
          "border-radius": "9999px",
          background: "var(--color-cyan)",
          "box-shadow": "0 0 8px var(--caret-glow-cyan)",
          display: "inline-block",
        }}
      />
      <span
        style={{
          "font-size": "10.5px",
          "font-weight": 600,
          "letter-spacing": "0.04em",
          color: "var(--color-cyan)",
          opacity: 0.85,
          "line-height": "1",
        }}
      >
        Transcribing…
      </span>
    </div>
  );
}
