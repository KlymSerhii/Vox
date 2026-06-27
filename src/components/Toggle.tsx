export function Toggle(props: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.checked}
      onClick={() => props.onChange(!props.checked)}
      class="relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full ring-1 ring-inset transition-colors"
      classList={{
        "bg-[var(--color-accent)] ring-transparent": props.checked,
        "bg-[var(--color-control)] ring-[var(--color-border-strong)]":
          !props.checked,
      }}
    >
      <span
        class="toggle-knob inline-block h-[18px] w-[18px] transform rounded-full transition-transform"
        classList={{
          "translate-x-[18px]": props.checked,
          "translate-x-[2px]": !props.checked,
        }}
      />
    </button>
  );
}
