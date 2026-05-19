import { createSignal, onCleanup, onMount, Show } from "solid-js";

export function ModeOverflowMenu(props: {
  onExport: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
}) {
  const [open, setOpen] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;

  function onDocClick(e: MouseEvent) {
    if (!containerRef) return;
    if (!containerRef.contains(e.target as Node)) setOpen(false);
  }

  onMount(() => document.addEventListener("mousedown", onDocClick));
  onCleanup(() => document.removeEventListener("mousedown", onDocClick));

  return (
    <div ref={containerRef} class="relative">
      <button
        type="button"
        aria-label="More actions"
        class="flex h-[28px] w-[28px] items-center justify-center rounded-[6px] bg-[var(--color-control)] text-[var(--color-ink-mute)] hover:bg-[var(--color-control-hover)] hover:text-[var(--color-ink)]"
        onClick={() => setOpen(!open())}
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="currentColor"
          aria-hidden="true"
        >
          <circle cx="5" cy="12" r="1.6" />
          <circle cx="12" cy="12" r="1.6" />
          <circle cx="19" cy="12" r="1.6" />
        </svg>
      </button>
      <Show when={open()}>
        <div
          class="absolute right-0 top-[32px] z-30 flex min-w-[160px] flex-col overflow-hidden rounded-[8px] ring-1 ring-[var(--color-border-strong)]"
          style={{
            background: "var(--color-surface-raised)",
            "backdrop-filter": "var(--glass-blur)",
            "-webkit-backdrop-filter": "var(--glass-blur)",
            "box-shadow": "var(--glass-shadow-stack-sm)",
          }}
          role="menu"
        >
          <MenuItem
            onClick={() => {
              setOpen(false);
              props.onDuplicate();
            }}
          >
            Duplicate
          </MenuItem>
          <MenuItem
            onClick={() => {
              setOpen(false);
              props.onExport();
            }}
          >
            Export…
          </MenuItem>
          <MenuItem
            danger
            onClick={() => {
              setOpen(false);
              props.onDelete();
            }}
          >
            Delete
          </MenuItem>
        </div>
      </Show>
    </div>
  );
}

function MenuItem(props: {
  children: any;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={props.onClick}
      class="px-3 py-1.5 text-left text-[12px] hover:bg-[var(--color-overlay)]"
      classList={{
        "text-[var(--color-ink)]": !props.danger,
        "text-[var(--color-danger)]": props.danger,
      }}
    >
      {props.children}
    </button>
  );
}
