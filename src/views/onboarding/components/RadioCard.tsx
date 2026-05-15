import { type JSX, Show } from "solid-js";

/// Shared visual recipe for the three radio-style cards in the onboarding
/// wizard: LanguageCard, PolishEngineCard, ModelCard. Renders a card with a
/// radio dot, title, subtitle, optional badge/meta rows, and an optional
/// expansion slot for nested form controls (e.g. the "Other language"
/// dropdown).
///
/// Element choice:
/// - When `children` is present we render a `<div role="radio">` because
///   nesting interactive form controls (`<select>`) inside a `<button>` is
///   invalid HTML and triggers focus/click oddities.
/// - Otherwise we render a `<button type="button" role="radio">` so the
///   browser handles keyboard activation natively and we get free
///   `disabled` semantics.
export function RadioCard(props: {
  selected: boolean;
  onSelect: () => void;
  disabled?: boolean;
  title: string;
  subtitle?: string;
  /// Pills slotted under the subtitle in their own flex row. Pass the
  /// whole fragment conditionally so the row is omitted entirely when
  /// there's nothing to show.
  badges?: JSX.Element;
  /// Secondary row below the badges. Currently unused by any migrated
  /// card; reserved for future per-card extras (e.g. inline notes).
  meta?: JSX.Element;
  /// Expansion content rendered below `meta` (e.g. a nested `<select>`).
  /// Forces the wrapper element to a `<div>` because `<button>` cannot
  /// host interactive form controls.
  children?: JSX.Element;
  /// Two of the three current callsites set the shared `liquid-card`
  /// surface; ModelCard intentionally does not. Defaults to true so
  /// migrations stay minimal.
  liquidCard?: boolean;
}) {
  const liquidCard = () => props.liquidCard ?? true;
  const hasChildren = () => props.children !== undefined && props.children !== null;
  const baseClass =
    "flex flex-col gap-1 rounded-[10px] bg-[var(--color-surface-raised)] px-3 py-2.5 text-left transition-[box-shadow,background-color]";

  // Renders the title + subtitle + badge/meta/children body. Shared
  // between the button and div code paths so the visual recipe lives in
  // exactly one place.
  const body = () => (
    <>
      <div class="flex items-center justify-between gap-2">
        <span class="text-[13px] font-medium text-[var(--color-ink)]">
          {props.title}
        </span>
        <span
          class="flex h-3.5 w-3.5 items-center justify-center rounded-full"
          classList={{
            "ring-1 ring-[var(--color-border-strong)]": !props.selected,
            "bg-[var(--color-accent)]": props.selected,
          }}
          aria-hidden="true"
        >
          <Show when={props.selected}>
            <span class="h-1.5 w-1.5 rounded-full bg-[var(--color-on-accent)]" />
          </Show>
        </span>
      </div>
      <Show when={props.subtitle}>
        <span class="text-[11px] text-[var(--color-ink-mute)]">
          {props.subtitle}
        </span>
      </Show>
      <Show when={props.badges}>
        <div class="mt-1 flex flex-wrap gap-1">{props.badges}</div>
      </Show>
      <Show when={props.meta}>{props.meta}</Show>
      <Show when={hasChildren()}>
        {/* Stop click bubbling so opening the dropdown doesn't re-fire
            the parent card's onSelect with the *previous* dropdown
            value. */}
        <div onClick={(e) => e.stopPropagation()}>{props.children}</div>
      </Show>
    </>
  );

  // Div path: children present means a `<select>` lives inside. We can't
  // use `<button>` (no interactive children) so we hand-roll role,
  // tabindex, and Enter/Space activation.
  return (
    <Show
      when={hasChildren()}
      fallback={
        <button
          type="button"
          role="radio"
          aria-checked={props.selected}
          disabled={props.disabled}
          onClick={props.onSelect}
          class={
            liquidCard()
              ? `liquid-card ${baseClass} disabled:cursor-not-allowed`
              : baseClass
          }
          classList={{
            "ring-1 ring-[var(--color-border)]":
              !props.selected && !props.disabled,
            "ring-2 ring-[var(--color-accent)]": props.selected,
            "opacity-50": !!props.disabled,
          }}
        >
          {body()}
        </button>
      }
    >
      <div
        role="radio"
        aria-checked={props.selected}
        tabindex={0}
        onClick={() => {
          if (props.disabled) return;
          props.onSelect();
        }}
        onKeyDown={(e) => {
          if (props.disabled) return;
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            props.onSelect();
          }
        }}
        class={
          liquidCard()
            ? `${baseClass} liquid-card cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-accent)]`
            : `${baseClass} cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-accent)]`
        }
        classList={{
          "ring-1 ring-[var(--color-border)]":
            !props.selected && !props.disabled,
          "ring-2 ring-[var(--color-accent)]": props.selected,
          "opacity-50 cursor-not-allowed": !!props.disabled,
        }}
      >
        {body()}
      </div>
    </Show>
  );
}
