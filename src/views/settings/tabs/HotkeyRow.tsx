import { createEffect, createSignal, For, type JSX, onCleanup, onMount, Show } from "solid-js";
import { HotkeyKeys } from "@/components/HotkeyKeys";
import {
  openAccessibilitySettings,
  type HotkeyDiagnostic,
} from "@/lib/ipc";
import {
  formatHotkey,
  isModsSpec,
  isTap2Spec,
  modsSpecFromFlags,
  parseSpec,
  specsEqual,
  tap2SpecFromModifier,
  validateHotkeySpec,
  type ValidationResult,
} from "@/lib/hotkey";
import {
  COMMON_CHORDS,
  DOUBLE_TAP_WINDOW_MS,
  EMPTY_HINT,
  EMPTY_SNAP,
  MODS_OK_HINT,
  TAP1_HINT,
  TAP2_OK_HINT,
  type CaptureMode,
  type CaptureSnapshot,
  type HotkeyKind,
} from "../constants";

export function HotkeyRow(props: {
  kind: HotkeyKind;
  label: string;
  hint: string;
  configured: string;
  defaultSpec: string;
  /// True for cancel / cycle: an empty spec means "off" and is allowed.
  /// PTT can't be off (the user would have no way to dictate), so reset
  /// always writes the default.
  allowEmpty: boolean;
  diagnostic: HotkeyDiagnostic | null;
  others: { kind: HotkeyKind; label: string; spec: string }[];
  /// Live result of `check_permissions().accessibility`. Polled at the
  /// tab level so all three rows share a single 1.5 s poll cycle.
  axGranted: boolean;
  /// External request to auto-enter capture (used by "Change other
  /// shortcut" conflict resolution).
  autoOpen?: boolean;
  onAutoOpened?: () => void;
  /// Set briefly after a sibling row's "Use this and clear that one"
  /// rewrites this row, so the keycap chip flashes once to confirm
  /// the change is visible.
  flash?: boolean;
  onChange: (v: string) => void;
  /// One-shot "Use this and clear that one" handler. Writes both rows
  /// in a single `setSettings` so the backend re-binds once.
  onResolveConflict: (self: string, other: HotkeyKind) => void;
  onChangeOther: (k: HotkeyKind) => void;
  /// Bubbles up "I'm capturing a `Mods:` or `Tap2:` spec right now" so
  /// the tab can gate the AX poll on the union of all three rows. The
  /// callback fires only when the value flips, not on every keystroke.
  onCapturingAxSpecChange?: (active: boolean) => void;
  extra?: JSX.Element;
  last?: boolean;
}) {
  const [mode, setMode] = createSignal<CaptureMode>({ kind: "idle" });
  let changeBtn: HTMLButtonElement | undefined;
  let captureBox: HTMLDivElement | undefined;
  let saveBtn: HTMLButtonElement | undefined;
  /// Repeat-debounce: same `e.code` within 50 ms is treated as autorepeat.
  let lastKey: { code: string; at: number } | null = null;
  /// Records the most recent clean release of a single modifier with
  /// nothing else held. A subsequent keydown of the same modifier
  /// within 400 ms — while the set is still empty — promotes to
  /// `Tap2:<Mod>`. Cleared whenever the captured set becomes non-empty
  /// or a different modifier is pressed.
  let lastSingleModRelease: {
    mod: "Cmd" | "Ctrl" | "Alt" | "Shift";
    at: number;
  } | null = null;

  const inCapture = () => mode().kind === "capturing";
  const snapOf = (m: CaptureMode): CaptureSnapshot =>
    m.kind === "capturing" ? m.snap : EMPTY_SNAP;

  const enterCapture = (snap: CaptureSnapshot = EMPTY_SNAP) => {
    lastKey = null;
    lastSingleModRelease = null;
    setMode({ kind: "capturing", snap });
    queueMicrotask(() => captureBox?.focus());
  };

  const exitToIdle = (restoreFocus: boolean) => {
    lastKey = null;
    lastSingleModRelease = null;
    setMode({ kind: "idle" });
    if (restoreFocus) queueMicrotask(() => changeBtn?.focus());
  };

  /// Run validation + conflict detection on a candidate spec and
  /// return the resulting snapshot. Cross-row conflict is evaluated
  /// here so the verdict line stays in lockstep with the set.
  const evaluate = (spec: string): CaptureSnapshot => {
    if (!spec) return EMPTY_SNAP;
    const v: ValidationResult = validateHotkeySpec(spec, props.kind, props.axGranted);
    if (v.kind === "invalid") {
      return { spec, verdict: { tone: "danger", message: v.reason } };
    }
    if (v.kind === "ok-needs-ax") {
      return {
        spec,
        verdict: {
          tone: "needs-ax",
          message: isTap2Spec(spec)
            ? "Needs Accessibility — Vox can't hear double-tap chords."
            : "Needs Accessibility — Vox can't hear modifier-only chords.",
        },
      };
    }
    const conflict = props.others.find((o) => specsEqual(o.spec, spec));
    if (conflict) {
      return {
        spec,
        verdict: {
          tone: "conflict",
          message: `Already used by ${conflict.label}.`,
          with: { kind: conflict.kind, label: conflict.label },
        },
      };
    }
    if (isTap2Spec(spec)) {
      return { spec, verdict: { tone: "ok", message: TAP2_OK_HINT } };
    }
    if (isModsSpec(spec)) {
      return { spec, verdict: { tone: "ok", message: MODS_OK_HINT } };
    }
    return { spec, verdict: { tone: "ok", message: "Looks good." } };
  };

  /// Live re-evaluate when AX flips while in capture so a `Mods:` /
  /// `Tap2:` spec unblocks Save without re-entering capture.
  createEffect(() => {
    const granted = props.axGranted;
    const m = mode();
    if (m.kind !== "capturing") return;
    const s = m.snap;
    if (!s.spec) return;
    const isAxSpec = isModsSpec(s.spec) || isTap2Spec(s.spec);
    if (!isAxSpec) return;
    const needsAx = s.verdict.tone === "needs-ax";
    if (needsAx && granted) {
      setMode({ kind: "capturing", snap: evaluate(s.spec) });
    } else if (!needsAx && !granted && s.verdict.tone === "ok") {
      setMode({ kind: "capturing", snap: evaluate(s.spec) });
    }
  });

  /// Tell the tab whether we're capturing a `Mods:` / `Tap2:` spec
  /// right now — gates the parent's AX poll.
  createEffect(() => {
    const m = mode();
    const active =
      m.kind === "capturing" &&
      !!m.snap.spec &&
      (isModsSpec(m.snap.spec) || isTap2Spec(m.snap.spec));
    props.onCapturingAxSpecChange?.(active);
  });
  onCleanup(() => {
    props.onCapturingAxSpecChange?.(false);
  });

  /// Document-level / window-blur listeners — single pair per row, like
  /// v1. The handlers no-op when idle.
  const onDocPointer = (e: MouseEvent) => {
    if (!inCapture()) return;
    if (captureBox && e.target instanceof Node && !captureBox.contains(e.target)) {
      exitToIdle(false);
    }
  };
  const onWinBlur = () => {
    if (inCapture()) exitToIdle(false);
  };

  onMount(() => {
    document.addEventListener("mousedown", onDocPointer, true);
    window.addEventListener("blur", onWinBlur);
  });
  onCleanup(() => {
    document.removeEventListener("mousedown", onDocPointer, true);
    window.removeEventListener("blur", onWinBlur);
  });

  createEffect(() => {
    if (!props.autoOpen) return;
    queueMicrotask(() => {
      changeBtn?.scrollIntoView({ behavior: "smooth", block: "center" });
      enterCapture();
      props.onAutoOpened?.();
    });
  });

  /// Translate a `KeyboardEvent.code` to a Tauri token (`M`, `Period`,
  /// `F13`, …) or `null` for keys outside the parser vocabulary.
  const baseTokenFromCode = (code: string): string | null => {
    if (/^Key[A-Z]$/.test(code)) return code.slice(3);
    if (/^Digit[0-9]$/.test(code)) return code.slice(5);
    if (/^F([1-9]|1[0-9]|20)$/.test(code)) return code;
    if (/^Arrow(Up|Down|Left|Right)$/.test(code)) return code;
    switch (code) {
      case "Period":
      case "Slash":
      case "Comma":
      case "Semicolon":
      case "Quote":
      case "Backquote":
      case "Minus":
      case "Equal":
      case "BracketLeft":
      case "BracketRight":
      case "Backslash":
      case "Backspace":
      case "Enter":
      case "Space":
      case "Delete":
      case "Home":
      case "End":
      case "PageUp":
      case "PageDown":
      case "Insert":
      case "Escape":
        return code;
      default:
        return null;
    }
  };

  const codeToModifier = (
    code: string,
  ): "Cmd" | "Ctrl" | "Alt" | "Shift" | null => {
    if (code === "MetaLeft" || code === "MetaRight" || code === "OSLeft" || code === "OSRight") {
      return "Cmd";
    }
    if (code === "ControlLeft" || code === "ControlRight") return "Ctrl";
    if (code === "AltLeft" || code === "AltRight") return "Alt";
    if (code === "ShiftLeft" || code === "ShiftRight") return "Shift";
    return null;
  };

  /// Read the *currently-held* modifiers off a KeyboardEvent. Used at
  /// every keydown to update the captured-so-far set.  Releases never
  /// drop modifiers — per PM spec §3a the set is captured-so-far.
  const heldFlags = (e: KeyboardEvent) => ({
    cmd: e.metaKey,
    ctrl: e.ctrlKey,
    alt: e.altKey,
    shift: e.shiftKey,
  });

  /// Merge a captured-so-far set with a fresh KeyboardEvent's held
  /// modifiers — releasing a modifier never drops it.
  const mergeFlags = (
    snap: CaptureSnapshot,
    e: KeyboardEvent,
  ): { cmd: boolean; ctrl: boolean; alt: boolean; shift: boolean } => {
    const p = parseSpec(snap.spec);
    const live = heldFlags(e);
    // Promote a held modifier into the set; never drop one.
    return {
      cmd: p.cmd || live.cmd,
      ctrl: p.ctrl || live.ctrl,
      alt: p.alt || live.alt,
      shift: p.shift || live.shift,
    };
  };

  const onCaptureKeyDown = (e: KeyboardEvent) => {
    // Block Tab during capture so Shift+Tab can't be smuggled in as a
    // binding.
    if (e.code === "Tab") {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    if (e.code === "Enter" || e.code === "NumpadEnter") {
      e.preventDefault();
      e.stopPropagation();
      save();
      return;
    }
    // Esc is always cancel-capture, even on the cancel row — Esc is
    // surfaced as an explicit suggestion chip there instead of being
    // bindable through the key-press path.
    if (e.code === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      exitToIdle(true);
      return;
    }
    const m = mode();
    if (m.kind !== "capturing") {
      e.preventDefault();
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    // Repeat debounce.
    if (lastKey && lastKey.code === e.code && Date.now() - lastKey.at < 50) {
      return;
    }
    lastKey = { code: e.code, at: Date.now() };

    const snap = m.snap;
    const wasEmpty = !snap.spec;
    const mod = codeToModifier(e.code);

    if (mod) {
      const now = Date.now();
      // Double-tap promotion: this is a keydown of the same modifier
      // we saw cleanly released within the window, with the set still
      // empty in between. PM spec §3c / §4.
      if (
        props.kind === "ptt" &&
        wasEmpty &&
        lastSingleModRelease &&
        lastSingleModRelease.mod === mod &&
        now - lastSingleModRelease.at < DOUBLE_TAP_WINDOW_MS
      ) {
        lastSingleModRelease = null;
        const spec = tap2SpecFromModifier(mod);
        setMode({ kind: "capturing", snap: evaluate(spec) });
        return;
      }
      // Any other modifier press invalidates the tap2 promotion path.
      lastSingleModRelease = null;
      // Compute the captured-so-far set as `Mods:<…>`. The browser's
      // KeyboardEvent already reflects every currently-held modifier
      // (e.altKey/metaKey/etc.), so this naturally handles the
      // "hold ⌥ then press ⌘" case.
      const flags = mergeFlags(snap, e);
      const spec = modsSpecFromFlags(flags);
      if (!spec) return;
      // While only a single modifier is freshly held with nothing else
      // before, show the TAP1 hint so users know about the double-tap
      // option. Once the user adds anything else, we drop back to the
      // normal "Mods:" verdict.
      const parsedNew = parseSpec(spec);
      const aloneCount =
        (parsedNew.cmd ? 1 : 0) +
        (parsedNew.ctrl ? 1 : 0) +
        (parsedNew.alt ? 1 : 0) +
        (parsedNew.shift ? 1 : 0);
      if (props.kind === "ptt" && wasEmpty && aloneCount === 1) {
        setMode({
          kind: "capturing",
          snap: {
            spec,
            verdict: { tone: "neutral", message: TAP1_HINT },
          },
        });
      } else {
        setMode({ kind: "capturing", snap: evaluate(spec) });
      }
      return;
    }

    // Non-modifier base key. Drop the tap tracker — a base key
    // disqualifies the double-tap branch.
    lastSingleModRelease = null;
    const tok = baseTokenFromCode(e.code);
    if (!tok) return;
    const flags = mergeFlags(snap, e);
    const parts: string[] = [];
    if (flags.cmd) parts.push("Cmd");
    if (flags.ctrl) parts.push("Ctrl");
    if (flags.alt) parts.push("Alt");
    if (flags.shift) parts.push("Shift");
    parts.push(tok);
    // Soft hint for the user mashing a second base key: keep last-wins
    // but tell them once.
    const oldParsed = parseSpec(snap.spec);
    const hasOldKey = oldParsed.shape === "single" && !!oldParsed.key;
    const spec = parts.join("+");
    const newSnap = evaluate(spec);
    if (
      hasOldKey &&
      newSnap.verdict.tone === "ok" &&
      oldParsed.key !== parseSpec(spec).key
    ) {
      const friendlyKey = parseSpec(spec).key;
      setMode({
        kind: "capturing",
        snap: {
          spec,
          verdict: {
            tone: "neutral",
            message: `One base key only — using ${friendlyKey}. Click Clear to start over.`,
          },
        },
      });
      return;
    }
    setMode({ kind: "capturing", snap: newSnap });
  };

  /// On modifier release we generally DO NOT drop it from the captured
  /// set — per design spec §1 the set is captured-so-far. The single
  /// exception: a quick lone-modifier tap on the PTT row, with no
  /// other modifier joining the set and no base key pressed, rewinds
  /// the set to empty so a follow-up tap of the same modifier within
  /// 400 ms can promote to `Tap2:<Mod>`. PM spec §3c / §4.
  const onCaptureKeyUp = (e: KeyboardEvent) => {
    if (!inCapture()) return;
    e.preventDefault();
    e.stopPropagation();
    const m = mode();
    if (m.kind !== "capturing") return;
    if (props.kind !== "ptt") return;
    const mod = codeToModifier(e.code);
    if (!mod) return;
    // Only when the OS reports no modifiers still held — otherwise the
    // user is mid-chord.
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) {
      lastSingleModRelease = null;
      return;
    }
    // Speculative-tap rewind: a captured set that consists of just
    // this lone modifier gets dropped so the next press of the same
    // mod within 400 ms can promote to Tap2.
    const parsed = parseSpec(m.snap.spec);
    if (parsed.shape !== "mods") {
      lastSingleModRelease = null;
      return;
    }
    const aloneCount =
      (parsed.cmd ? 1 : 0) +
      (parsed.ctrl ? 1 : 0) +
      (parsed.alt ? 1 : 0) +
      (parsed.shift ? 1 : 0);
    const matchesReleased =
      (mod === "Cmd" && parsed.cmd) ||
      (mod === "Ctrl" && parsed.ctrl) ||
      (mod === "Alt" && parsed.alt) ||
      (mod === "Shift" && parsed.shift);
    if (aloneCount === 1 && matchesReleased) {
      lastSingleModRelease = { mod, at: Date.now() };
      setMode({ kind: "capturing", snap: EMPTY_SNAP });
    } else {
      lastSingleModRelease = null;
    }
  };

  const pickCommon = (spec: string) => {
    lastSingleModRelease = null;
    setMode({ kind: "capturing", snap: evaluate(spec) });
  };

  const clear = () => {
    lastSingleModRelease = null;
    setMode({ kind: "capturing", snap: EMPTY_SNAP });
  };

  const save = () => {
    const m = mode();
    if (m.kind !== "capturing") return;
    const s = m.snap;
    if (s.verdict.tone !== "ok") return;
    props.onChange(s.spec);
    exitToIdle(true);
  };

  const reset = () => {
    props.onChange(props.defaultSpec);
  };

  // Diagnostic chips — kept from v1 so the fallback signal stays visible.
  const showBoundOverride = () => {
    const d = props.diagnostic;
    return d && d.fell_back && d.bound && !specsEqual(d.bound, props.configured);
  };
  const failed = () => {
    const d = props.diagnostic;
    return d != null && !d.bound && props.configured !== "";
  };

  /// Move focus to Save when the verdict flips to `ok`, per design §6.
  createEffect(() => {
    const m = mode();
    if (m.kind !== "capturing") return;
    if (m.snap.verdict.tone === "ok") {
      queueMicrotask(() => saveBtn?.focus());
    }
  });

  return (
    <div
      class="flex items-start justify-between gap-4 px-4 py-3"
      classList={{
        "border-b border-[var(--color-border)]": !props.last,
      }}
    >
      <div class="flex min-w-0 flex-col gap-1">
        <div class="flex items-center gap-2">
          <span class="text-[13px] text-[var(--color-ink)]">{props.label}</span>
          <Show when={showBoundOverride()}>
            <span class="rounded-full bg-[var(--color-accent)]/20 px-2 py-0.5 text-[10px] text-[var(--color-accent)]">
              Bound to {formatHotkey(props.diagnostic?.bound)}
            </span>
          </Show>
          <Show when={failed()}>
            <span class="rounded-full bg-[var(--color-danger)]/20 px-2 py-0.5 text-[10px] text-[var(--color-danger)]">
              Not bound
            </span>
          </Show>
        </div>
        <span class="text-[11px] text-[var(--color-ink-mute)]">
          {props.hint}
        </span>
        <Show when={showBoundOverride()}>
          <span class="text-[11px] text-[var(--color-ink-faint)]">
            Configured {formatHotkey(props.configured)} couldn't be bound on this
            Mac — Vox is using {formatHotkey(props.diagnostic?.bound)}.
          </span>
        </Show>
        <Show when={failed() && props.diagnostic?.error}>
          <span class="text-[11px] text-[var(--color-danger)]">
            {props.diagnostic?.error}
          </span>
        </Show>
      </div>
      <div class="flex shrink-0 flex-col items-end gap-2">
        <Show when={mode().kind === "idle"}>
          <div class="flex flex-col items-end gap-1">
            <Show
              when={props.configured !== ""}
              fallback={
                <span
                  class="text-[11px] italic text-[var(--color-ink-faint)]"
                  classList={{ "animate-flash-done": !!props.flash }}
                >
                  Off
                </span>
              }
            >
              <span classList={{ "animate-flash-done": !!props.flash }}>
                <HotkeyKeys spec={props.configured} />
              </span>
            </Show>
            <div class="flex items-center gap-1.5">
              <Show when={failed()}>
                <button
                  type="button"
                  onClick={() => {
                    // Re-enter capture pre-populated with the failing
                    // spec so the user sees what was refused and can
                    // re-record over it. Per PM §5b.
                    enterCapture(evaluate(props.configured));
                  }}
                  class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:bg-[var(--color-accent-strong)]"
                >
                  Try another
                </button>
              </Show>
              <button
                ref={changeBtn}
                type="button"
                onClick={() => enterCapture()}
                class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] hover:bg-[var(--color-control-hover)]"
              >
                Change
              </button>
              <button
                type="button"
                onClick={reset}
                disabled={specsEqual(props.configured, props.defaultSpec)}
                class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] hover:bg-[var(--color-control-hover)] disabled:opacity-40 disabled:hover:bg-[var(--color-control)]"
                title={`Reset to ${formatHotkey(props.defaultSpec)}`}
              >
                Reset
              </button>
              <Show when={props.allowEmpty && props.configured !== ""}>
                <button
                  type="button"
                  onClick={() => props.onChange("")}
                  class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] text-[var(--color-ink-mute)] hover:bg-[var(--color-control-hover)] hover:text-[var(--color-ink)]"
                  title="Disable this shortcut"
                >
                  Off
                </button>
              </Show>
            </div>
          </div>
        </Show>

        <Show when={inCapture()}>
          {(() => {
            const snap = () => snapOf(mode());
            const v = () => snap().verdict;
            const tone = () => v().tone;
            const canSave = () => tone() === "ok";
            const verdictMessage = () => {
              const cur = v();
              if (cur.tone === "empty") return EMPTY_HINT;
              return "message" in cur ? cur.message : "";
            };
            const verdictClass = () => {
              const t = tone();
              if (t === "ok" || t === "neutral") return "text-[var(--color-ink-mute)]";
              if (t === "empty") return "text-[var(--color-ink)] animate-pulse-mic";
              return "text-[var(--color-danger)]";
            };
            const verdictId = `hotkey-capture-${props.kind}-verdict`;
            return (
              <div
                ref={captureBox}
                role="dialog"
                aria-label={`Capture ${props.label} shortcut`}
                aria-describedby={verdictId}
                tabIndex={-1}
                onKeyDown={onCaptureKeyDown}
                onKeyUp={onCaptureKeyUp}
                class="liquid-glass flex w-[260px] flex-col gap-2 rounded-[10px] p-2.5 ring-1 ring-[var(--color-accent)] outline-none animate-pill-in"
              >
                {/* Slot 1 — keycaps or placeholder */}
                <div class="flex min-h-[20px] flex-wrap items-center gap-1">
                  <Show
                    when={!!snap().spec}
                    fallback={
                      <span class="text-[11px] italic text-[var(--color-ink-faint)]">
                        —
                      </span>
                    }
                  >
                    <HotkeyKeys spec={snap().spec} />
                  </Show>
                </div>
                {/* Slot 2 — verdict */}
                <span id={verdictId} class={`text-[11px] ${verdictClass()}`} aria-live="polite">
                  {verdictMessage()}
                </span>
                {/* Slot 3 — permission / conflict resolution */}
                <Show when={tone() === "needs-ax"}>
                  <div class="flex items-center gap-1.5">
                    <button
                      type="button"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        void openAccessibilitySettings();
                      }}
                      class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] hover:bg-[var(--color-control-hover)]"
                    >
                      Open System Settings
                    </button>
                  </div>
                </Show>
                <Show when={tone() === "conflict"}>
                  {(() => {
                    const cur = v();
                    if (cur.tone !== "conflict") return null;
                    return (
                      <div class="flex flex-col gap-1">
                        <button
                          type="button"
                          onMouseDown={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            props.onResolveConflict(snap().spec, cur.with.kind);
                            exitToIdle(true);
                          }}
                          class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:bg-[var(--color-accent-strong)]"
                        >
                          Use this & clear that one
                        </button>
                        <button
                          type="button"
                          onMouseDown={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            const target = cur.with.kind;
                            exitToIdle(false);
                            props.onChangeOther(target);
                          }}
                          class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] hover:bg-[var(--color-control-hover)]"
                        >
                          Change other shortcut
                        </button>
                      </div>
                    );
                  })()}
                </Show>
                {/* Slot 4 — Save / Clear / Cancel */}
                <div class="flex items-center gap-1.5">
                  <button
                    ref={saveBtn}
                    type="button"
                    disabled={!canSave()}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      save();
                    }}
                    title={
                      tone() === "needs-ax"
                        ? "Needs Accessibility — open System Settings to grant."
                        : undefined
                    }
                    class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:bg-[var(--color-accent-strong)] disabled:opacity-40 disabled:hover:bg-[var(--color-accent)]"
                  >
                    Save
                  </button>
                  <button
                    type="button"
                    disabled={!snap().spec}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      clear();
                    }}
                    class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] hover:bg-[var(--color-control-hover)] disabled:opacity-40 disabled:hover:bg-[var(--color-control)]"
                  >
                    Clear
                  </button>
                  <button
                    type="button"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      exitToIdle(true);
                    }}
                    class="rounded-[6px] px-3 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
                  >
                    Cancel
                  </button>
                </div>
              </div>
            );
          })()}
        </Show>
        <Show when={inCapture()}>
          {/* Picker matches the capture popover width (260px) above so the
              chord chips form a tidy column under it instead of a ragged
              flex-wrap row. Two columns keep the tallest list (8 chips
              for PTT) at 4 rows. */}
          <div class="flex w-[260px] flex-col gap-1.5">
            <span class="text-[10px] uppercase tracking-[0.06em] text-[var(--color-ink-faint)]">
              Or pick
            </span>
            <div class="grid grid-cols-2 gap-1">
              <For
                each={COMMON_CHORDS.filter(
                  (c) =>
                    (!c.cancelOnly || props.kind === "cancel") &&
                    (!c.pttOnly || props.kind === "ptt"),
                )}
              >
                {(c) => (
                  <button
                    type="button"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      pickCommon(c.spec);
                    }}
                    class="flex items-center justify-center rounded-[6px] bg-[var(--color-control)] px-2 py-1 text-[10px] hover:bg-[var(--color-control-hover)]"
                    classList={{ "col-span-2": !!c.wide }}
                    aria-label={`Use ${formatHotkey(c.spec)}`}
                  >
                    <HotkeyKeys spec={c.spec} />
                  </button>
                )}
              </For>
            </div>
          </div>
        </Show>
        <Show when={!inCapture()}>{props.extra}</Show>
      </div>
    </div>
  );
}
