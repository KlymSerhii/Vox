/// Three staggered pulsing dots shown in the body area while no streamed
/// text has arrived yet. Gives a visible loading signal during the
/// Whisper startup gap and the LLM TTFT gap.
export function TypingDots(props: { color: string }) {
  const delays = ["0ms", "120ms", "240ms"];
  return (
    <div
      class="flex items-center justify-center"
      style={{ gap: "5px", padding: "4px 0 2px" }}
      aria-hidden="true"
    >
      {delays.map((delay) => (
        <span
          class="typing-dot"
          style={{
            width: "5px",
            height: "5px",
            "border-radius": "9999px",
            background: props.color,
            display: "inline-block",
            "animation-delay": delay,
          }}
        />
      ))}
    </div>
  );
}
