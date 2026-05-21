import { Show } from "solid-js";

/// Single-slot streaming text. Wraps to multiple lines so long
/// transcripts and polishes are not cropped to a single 26px ribbon;
/// the parent pill window grows in lockstep via the ResizeObserver
/// hook in PillView.
///
/// Four visual states share this slot:
///   1. Transcript streaming: cyan caret + glow blinking after text,
///      and a continuous cyan `pill-shimmer` sweep across the body
///      itself so the active transcription reads as alive — not just
///      the trailing caret. (Brief: "I want to always see this cyan
///      shimmer, which goes from left to right during transcription,
///      in addition to the existing caret".)
///   2. Waiting for polish:   `pill-shimmer` sweep recolours to
///      violet — the transcript freeze between final-transcript and
///      the LLM's first token gets the polish tint so the gap reads
///      as active processing instead of a frozen pill.
///   3. Polish streaming:     violet caret + glow, plain ink text.
///   4. (Reduced motion):     CSS overrides drop the shimmer
///      animation and hand the body back a solid ink colour, and the
///      caret freezes solid. This file doesn't need to branch on it —
///      the shimmer/caret CSS classes both opt out under
///      prefers-reduced-motion.
export function StreamingText(props: {
  text: string;
  mode: "transcript" | "polish";
  waitingForPolish: boolean;
  /// True while we are actively in the `transcribing` stage with a
  /// transcript stream still flowing (i.e. *before* the polish stream
  /// starts). Drives the cyan body shimmer. Mutually exclusive with
  /// `waitingForPolish` by construction in the parent (waitingForPolish
  /// requires `effectiveState === "polishing"`, transcribingStreaming
  /// requires `effectiveState === "transcribing"`).
  transcribingStreaming: boolean;
  /// True while the live-transcript opt-in is rendering the
  /// speculative buffer during the listening phase. The caret + body
  /// shimmer tint to coral (matching the listening meter colour) so
  /// the stage flip listening → transcribing reads as a colour
  /// hand-off rather than two strangers in the same slot.
  listening?: boolean;
}) {
  // Caret colour layers: listening (coral) > polish (violet) >
  // transcribing (cyan). Listening wins because the user is still
  // mid-utterance — the speculative text is *living*, not settled,
  // and the warm tint marks that.
  const caretColor = () =>
    props.listening
      ? "var(--color-coral)"
      : props.mode === "polish"
        ? "var(--color-violet)"
        : "var(--color-cyan)";
  // Glow points at theme-aware tokens so light vs dark themes pick
  // up rgbas tuned for each surface. Falls back to a cyan-strength
  // glow for the listening case so the coral caret stays haloed.
  const caretGlow = () =>
    props.listening
      ? "var(--caret-glow-coral, var(--caret-glow-cyan))"
      : props.mode === "polish"
        ? "var(--caret-glow-violet)"
        : "var(--caret-glow-cyan)";
  // Shimmer runs whenever live text is flowing — listening,
  // transcribing, or polishing. The tint follows the stage so the
  // band visibly carries the active phase's colour.
  const shimmerActive = () =>
    props.listening ||
    props.waitingForPolish ||
    props.transcribingStreaming ||
    props.mode === "polish";
  const shimmerTint = () =>
    props.listening
      ? "var(--color-coral)"
      : props.waitingForPolish || props.mode === "polish"
        ? "var(--color-violet)"
        : "var(--color-cyan)";
  return (
    <div
      class="flex w-full select-none items-start"
      style={{
        // Left-aligned for multi-line readability. Right alignment
        // works for one-liners but flips reading direction at every
        // wrap, hostile for anything past ~10 words.
        "justify-content": "flex-start",
      }}
      aria-live="off"
    >
      <span
        class="text-[11px] font-medium"
        classList={{ "pill-shimmer": shimmerActive() }}
        style={{
          color: "var(--color-ink)",
          "letter-spacing": "0.005em",
          opacity: 0.95,
          // 1.4 line-height at 11px text yields ~15px per line; a
          // 5-line polish lands in ~75px of pill height.
          "line-height": "1.4",
          // pre-wrap (not normal) so LLM-polished text that contains
          // \n\n paragraphs, numbered lists, or bullet points keeps
          // its line breaks instead of collapsing to a single visual
          // line. Wrapping behaviour stays the same — soft wrap at
          // word boundaries, with `word-break: break-word` to break
          // absurdly long tokens (pasted URLs) before they escape
          // the right rim. The pill's ResizeObserver picks up the
          // new height naturally as multi-line content expands and
          // pushes a `pill_resize` to grow the OS window in lockstep.
          "white-space": "pre-wrap",
          "word-break": "break-word",
          padding: "2px 0",
          width: "100%",
          transition: "color 180ms ease-out",
          // Shimmer ramp: ink -> tint -> ink. Tint flips to cyan
          // during transcription streaming and violet during the
          // polish-gap, so the band visibly carries the stage colour.
          "--shimmer-base": "var(--color-ink)",
          "--shimmer-tint": shimmerTint(),
        }}
      >
        {props.text || " "}
        {/* Inline blinking caret stays attached to the trailing
            character so on multi-line wraps it sits at the end of
            the last line. Hidden during the polish-gap shimmer
            because the shimmer itself is the activity indicator —
            but stays VISIBLE during transcribing-streaming shimmer,
            since the caret marks where the next token will land. */}
        <Show when={!props.waitingForPolish}>
          <span
            class="streaming-caret"
            aria-hidden="true"
            style={{
              "background-color": caretColor(),
              "margin-left": "2px",
              "margin-right": "1px",
              "vertical-align": "text-bottom",
              display: "inline-block",
              width: "2px",
              height: "16px",
              "border-radius": "1px",
              // CSS class .streaming-caret reads --caret-glow for
              // its box-shadow, so flipping the variable per-state
              // gets us the cyan/violet halo without animation forks.
              "--caret-glow": caretGlow(),
            }}
          />
        </Show>
      </span>
    </div>
  );
}
