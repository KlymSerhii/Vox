// Shared outer shell for the row-style cards in Settings. Two sizes mirror the
// two visual tiers used in this view: `md` (the engine cards with the larger
// 14px corner + raised surface tint) and `sm` (the per-row liquid cards with
// a tighter 10px corner). Per-site padding flows through `class` because the
// two sm sites use two different paddings.
//
// `disabled` is honoured by the md variant: it fades the card and sets
// aria-disabled so SttEngineCard's existing semantics survive the migration.
export function RaisedCard(props: {
  children: any;
  class?: string;
  size?: "sm" | "md";
  disabled?: boolean;
}) {
  const base = () =>
    props.size === "sm"
      ? "liquid-card mb-3 rounded-[10px] ring-1 ring-[var(--color-border)]"
      : "mb-3 rounded-[14px] bg-[var(--color-surface-raised)] p-3 ring-1 ring-[var(--color-border)]";
  return (
    <div
      class={`${base()} ${props.class ?? ""}`.trim()}
      classList={{ "opacity-60": props.disabled }}
      aria-disabled={props.disabled ? true : undefined}
    >
      {props.children}
    </div>
  );
}
