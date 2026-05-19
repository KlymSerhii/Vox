export function StaticValue(props: { children: any }) {
  return (
    <span class="text-[12px] font-medium text-[var(--color-ink-mute)]">
      {props.children}
    </span>
  );
}
