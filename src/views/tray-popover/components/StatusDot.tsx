export function StatusDot(props: { tone: "ok" | "off" | "warn" }) {
  const color =
    props.tone === "ok"
      ? "var(--color-success)"
      : props.tone === "warn"
        ? "var(--color-warning)"
        : "var(--color-ink-faint)";
  return (
    <span
      class="inline-block h-2 w-2 rounded-full"
      style={{ background: color }}
    />
  );
}
