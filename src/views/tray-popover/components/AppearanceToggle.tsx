export function AppearanceToggle(props: {
  value: "light" | "dark";
  onChange: (v: "light" | "dark") => void;
}) {
  return (
    <div
      class="inline-flex items-center rounded-full p-[3px] ring-1 ring-inset ring-[var(--color-border)]"
      style={{ background: "var(--color-control)" }}
      role="radiogroup"
      aria-label="Appearance"
    >
      <AppearanceHalf
        active={props.value === "light"}
        label="Light"
        onClick={() => props.onChange("light")}
      >
        <SunGlyph />
      </AppearanceHalf>
      <AppearanceHalf
        active={props.value === "dark"}
        label="Dark"
        onClick={() => props.onChange("dark")}
      >
        <MoonGlyph />
      </AppearanceHalf>
    </div>
  );
}

function AppearanceHalf(props: {
  active: boolean;
  label: string;
  onClick: () => void;
  children: any;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={props.active}
      aria-label={props.label}
      class="inline-flex h-6 w-7 items-center justify-center rounded-full transition-all"
      classList={{
        "hover:text-[var(--color-ink)]": !props.active,
      }}
      style={{
        background: props.active ? "var(--color-surface-raised)" : "transparent",
        "backdrop-filter": props.active ? "var(--glass-blur)" : "none",
        "-webkit-backdrop-filter": props.active ? "var(--glass-blur)" : "none",
        "box-shadow": props.active ? "var(--glass-shadow-stack-sm)" : "none",
        color: props.active
          ? "var(--color-ink)"
          : "var(--color-ink-mute)",
      }}
      onClick={props.onClick}
    >
      {props.children}
    </button>
  );
}

function SunGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="3.5" />
      <line x1="12" y1="2" x2="12" y2="4" />
      <line x1="12" y1="20" x2="12" y2="22" />
      <line x1="2" y1="12" x2="4" y2="12" />
      <line x1="20" y1="12" x2="22" y2="12" />
      <line x1="4.9" y1="4.9" x2="6.3" y2="6.3" />
      <line x1="17.7" y1="17.7" x2="19.1" y2="19.1" />
      <line x1="4.9" y1="19.1" x2="6.3" y2="17.7" />
      <line x1="17.7" y1="6.3" x2="19.1" y2="4.9" />
    </svg>
  );
}

function MoonGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" />
    </svg>
  );
}
