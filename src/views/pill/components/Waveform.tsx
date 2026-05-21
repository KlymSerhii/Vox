import { edgeEnvelope } from "../helpers";

export function Waveform(props: { levels: number[]; color: string }) {
  // Waveform spans the full pill width; the edge-envelope opacity is
  // what the eye reads as "the wave fades into the pill" instead of a
  // hard cluster floating in the middle with visible empty padding on
  // the sides.
  return (
    <div
      class="flex h-full w-full items-center justify-between"
      aria-hidden="true"
    >
      {props.levels.map((level, i) => {
        // Gamma 0.65 opens up the bottom of the range — Whisper-grade
        // RMS spends most of its life in [0, 0.3].
        const shaped = Math.pow(level, 0.65);
        // Sine bell across the bar index. 0 at edges, 1 at the
        // middle — both height and opacity ride this so the wave
        // visually disappears into the pill rim.
        const env = edgeEnvelope(i, props.levels.length);
        const h = Math.max(
          1.5,
          Math.round((0.22 + shaped * 0.78) * 18 * (0.4 + 0.6 * env)),
        );
        const opacity = (0.32 + shaped * 0.68) * env;
        return (
          <div
            style={{
              width: "1.5px",
              height: `${h}px`,
              "border-radius": "1.5px",
              background: props.color,
              opacity,
              transition: "height 110ms ease-out, opacity 180ms ease-out",
            }}
          />
        );
      })}
    </div>
  );
}
