export function TabIcon(props: { name: string; active: boolean }) {
  const fill = () =>
    props.active ? "var(--color-accent)" : "var(--color-ink-mute)";
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke={fill()}
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.name === "sliders" && (
        <>
          <line x1="4" y1="21" x2="4" y2="14" />
          <line x1="4" y1="10" x2="4" y2="3" />
          <line x1="12" y1="21" x2="12" y2="12" />
          <line x1="12" y1="8" x2="12" y2="3" />
          <line x1="20" y1="21" x2="20" y2="16" />
          <line x1="20" y1="12" x2="20" y2="3" />
          <line x1="1" y1="14" x2="7" y2="14" />
          <line x1="9" y1="8" x2="15" y2="8" />
          <line x1="17" y1="16" x2="23" y2="16" />
        </>
      )}
      {props.name === "keyboard" && (
        <>
          <rect x="2" y="6" width="20" height="12" rx="2" />
          <line x1="6" y1="10" x2="6" y2="10" />
          <line x1="10" y1="10" x2="10" y2="10" />
          <line x1="14" y1="10" x2="14" y2="10" />
          <line x1="18" y1="10" x2="18" y2="10" />
          <line x1="7" y1="14" x2="17" y2="14" />
        </>
      )}
      {props.name === "key" && (
        <>
          <circle cx="8" cy="15" r="4" />
          <line x1="10.8" y1="12.2" x2="21" y2="2" />
          <line x1="17" y1="6" x2="21" y2="10" />
        </>
      )}
      {props.name === "book" && (
        <>
          <path d="M4 19.5V4a2 2 0 0 1 2-2h13v18H6a2 2 0 0 0-2 2" />
          <line x1="8" y1="7" x2="15" y2="7" />
        </>
      )}
      {props.name === "lock" && (
        <>
          <rect x="4" y="11" width="16" height="10" rx="2" />
          <path d="M8 11V7a4 4 0 0 1 8 0v4" />
        </>
      )}
      {props.name === "info" && (
        <>
          <circle cx="12" cy="12" r="9" />
          <line x1="12" y1="16" x2="12" y2="12" />
          <line x1="12" y1="8" x2="12.01" y2="8" />
        </>
      )}
      {props.name === "cube" && (
        <>
          <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
          <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
          <line x1="12" y1="22.08" x2="12" y2="12" />
        </>
      )}
      {props.name === "modes" && (
        <>
          <circle cx="12" cy="12" r="9" />
          <path d="M12 3v18" />
          <path d="M3 12h18" />
        </>
      )}
      {props.name === "checklist" && (
        <>
          <polyline points="3 6 4.5 7.5 7 5" />
          <polyline points="3 12 4.5 13.5 7 11" />
          <line x1="11" y1="6" x2="21" y2="6" />
          <line x1="11" y1="12" x2="21" y2="12" />
          <line x1="11" y1="18" x2="21" y2="18" />
        </>
      )}
    </svg>
  );
}
