export function Card(props: { children: any }) {
  return (
    <div class="liquid-card mb-5 overflow-hidden rounded-[14px] ring-1 ring-[var(--color-border)]">
      {props.children}
    </div>
  );
}
