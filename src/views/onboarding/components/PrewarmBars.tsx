import { For } from "solid-js";

/// Three accent-coloured bars that bounce vertically in stagger. The
/// 130 ms offset between bars puts the wave at ~1/8 cycle apart, which
/// is enough to read as a travelling wave but tight enough that all
/// three bars share visible motion at any given moment. Heights are
/// driven by the keyframe in styles.css so a single CSS variable can
/// tune cadence later.
export function PrewarmBars() {
  return (
    <div
      aria-hidden="true"
      class="flex h-5 w-7 shrink-0 items-center justify-between"
    >
      <For each={[0, 130, 260]}>
        {(delay) => (
          <span
            class="prewarm-bar h-full w-[3px] rounded-full bg-[var(--color-accent)]"
            style={{ "animation-delay": `${delay}ms` }}
          />
        )}
      </For>
    </div>
  );
}
