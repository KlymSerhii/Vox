import { createUniqueId, type JSX } from "solid-js";

interface VoxLogoProps {
  size: number;
  /** Below ~32px the speech-bubble tail muddies; the compact form
   * drops it. Auto-detects from `size` when omitted. */
  compact?: boolean;
  class?: string;
  style?: JSX.CSSProperties;
  ariaLabel?: string;
}

export function VoxLogo(props: VoxLogoProps) {
  // Unique per instance so two logos on one page (sidebar + About hero)
  // don't collide on `url(#vox-bg)` references.
  const uid = createUniqueId();
  const bgId = `vox-bg-${uid}`;
  const rimId = `vox-rim-${uid}`;
  const innerId = `vox-inner-${uid}`;
  const compact = () => props.compact ?? props.size < 32;
  return (
    <svg
      width={props.size}
      height={props.size}
      viewBox="0 0 48 48"
      class={props.class}
      style={props.style}
      role="img"
      aria-label={props.ariaLabel ?? "Vox"}
    >
      <defs>
        <linearGradient
          id={bgId}
          x1="0"
          y1="0"
          x2="48"
          y2="48"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0" stop-color="#FFB088" />
          <stop offset="1" stop-color="#FF7A5C" />
        </linearGradient>
        <linearGradient
          id={rimId}
          x1="24"
          y1="0"
          x2="24"
          y2="24"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.35" />
          <stop offset="1" stop-color="#FFFFFF" stop-opacity="0" />
        </linearGradient>
        <filter id={innerId} x="-10%" y="-10%" width="120%" height="120%">
          <feGaussianBlur in="SourceAlpha" stdDeviation="3" />
          <feOffset dy="3" />
          <feComposite
            in2="SourceAlpha"
            operator="arithmetic"
            k2="-1"
            k3="1"
            result="shadowDiff"
          />
          <feFlood flood-color="#A33616" flood-opacity="0.18" />
          <feComposite in2="shadowDiff" operator="in" />
          <feComposite in2="SourceGraphic" operator="over" />
        </filter>
      </defs>
      <path
        d="M24 0C10.7 0 0 10.7 0 24s10.7 24 24 24 24-10.7 24-24S37.3 0 24 0Z"
        fill={`url(#${bgId})`}
        filter={`url(#${innerId})`}
      />
      <path
        d="M24 0C10.7 0 0 10.7 0 24s10.7 24 24 24 24-10.7 24-24S37.3 0 24 0Z"
        fill="none"
        stroke={`url(#${rimId})`}
        stroke-width="1"
      />
      {/* Curly quote — left and right bowls. Always rendered. */}
      <path
        d="M16.2 14.5c-3.4 1.5-5.5 4.5-5.5 8.2 0 2.8 1.6 4.7 4 4.7 2.3 0 3.9-1.6 3.9-3.7 0-1.8-1.1-3-2.7-3.3.2-1.7 1.4-3.3 3.3-4.4.7-.4.9-1.1.5-1.6-.4-.5-1.1-.5-1.9-.1Zm6.3-.5L17.7 33.4c-.2.8.3 1.5 1.2 1.5h2.1c.7 0 1.3-.4 1.4-1.1l4.2-19c.2-.7-.2-1.3-.9-1.3h-2.6c-.5 0-.9.2-1 .5Z"
        fill="#FFF4E8"
      />
      {/* Speech-bubble tail. Dropped on compact form where it would mud. */}
      {!compact() && (
        <path
          d="M19.1 31.6 15.3 36.7c-.4.5.1 1.2.7 1l5.2-1.8c.5-.2.6-.8.2-1.2l-1.4-3.3c-.3-.6-1.2-.6-1.5.2Z"
          fill="#FFF4E8"
        />
      )}
    </svg>
  );
}
