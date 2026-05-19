import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import {
  checkPermissions,
  diagnoseHotkeys,
  getActiveHotkey,
  onSettingsChanged,
  type HotkeyDiagnostic,
  type Settings,
} from "@/lib/ipc";
import {
  formatHotkey,
  isModsSpec,
  isTap2Spec,
} from "@/lib/hotkey";
import { Card } from "../components/Card";
import { SectionHeader } from "../components/SectionHeader";
import {
  DEFAULT_CANCEL_HOTKEY,
  DEFAULT_MODE_CYCLE_HOTKEY,
  DEFAULT_PUSH_TO_TALK,
  type HotkeyKind,
} from "../constants";
import { pttHint } from "../helpers";
import { HotkeyRow } from "./HotkeyRow";

export function HotkeysTab(props: {
  s: Settings;
  update: (p: Partial<Settings>) => void;
}) {
  const [diags, setDiags] = createSignal<Map<string, HotkeyDiagnostic>>(
    new Map(),
  );
  const [testStatus, setTestStatus] = createSignal<string | null>(null);
  // When a cross-row collision is resolved with "Change other shortcut",
  // we set this so the target row auto-enters capture on next render.
  const [autoCapture, setAutoCapture] = createSignal<HotkeyKind | null>(null);
  // When a conflict is resolved with "Use this and clear that one", we
  // briefly flag the cleared row so its keycap chip flashes —
  // otherwise the user only notices the row they kept and may miss
  // that the other one was rewritten. Per design spec §5.
  const [flashRow, setFlashRow] = createSignal<HotkeyKind | null>(null);
  /// Cached `AXIsProcessTrusted()` result. Polled every 1.5 s from
  /// `check_permissions` so a grant in System Settings unlocks Save
  /// without re-entering capture. Optimistic default (`true`) so a
  /// slow first probe doesn't flicker "needs AX" for users who never
  /// touch mod-only / double-tap.
  const [axGranted, setAxGranted] = createSignal<boolean>(true);
  let flashTimer: ReturnType<typeof setTimeout> | null = null;
  const triggerFlash = (k: HotkeyKind) => {
    if (flashTimer != null) clearTimeout(flashTimer);
    setFlashRow(k);
    flashTimer = setTimeout(() => {
      setFlashRow(null);
      flashTimer = null;
    }, 220);
  };
  onCleanup(() => {
    if (flashTimer != null) clearTimeout(flashTimer);
  });

  async function refreshDiagnostics() {
    try {
      const r = await diagnoseHotkeys();
      const map = new Map<string, HotkeyDiagnostic>();
      for (const e of r.entries) map.set(e.purpose, e);
      setDiags(map);
    } catch (e) {
      console.warn("diagnoseHotkeys failed:", e);
    }
  }

  async function refreshAx() {
    try {
      const p = await checkPermissions();
      setAxGranted(p.accessibility);
    } catch (e) {
      console.warn("checkPermissions failed:", e);
    }
  }

  let unlistenSettings: (() => void) | null = null;
  let axPoll: ReturnType<typeof setInterval> | null = null;
  // Per-row "I'm capturing a Mods:/Tap2: spec right now" signal. Rows
  // bubble this up via `onCapturingAxSpecChange`; the poll is gated on
  // the union below.
  const [rowCapturingAx, setRowCapturingAx] = createSignal<Set<HotkeyKind>>(
    new Set(),
  );
  const setRowAx = (k: HotkeyKind, active: boolean) => {
    setRowCapturingAx((prev) => {
      const had = prev.has(k);
      if (had === active) return prev;
      const next = new Set(prev);
      if (active) next.add(k);
      else next.delete(k);
      return next;
    });
  };

  /// A row "needs AX answers" when it's mid-capture forming a
  /// `Mods:`/`Tap2:` spec, OR when a saved `Mods:`/`Tap2:` spec is
  /// configured and AX is currently denied. When no row needs the
  /// answer, the poll does not run.
  const pollActive = createMemo(() => {
    if (rowCapturingAx().size > 0) return true;
    if (!axGranted()) {
      const specs = [
        props.s.hotkey,
        props.s.cancel_hotkey,
        props.s.mode_cycle_hotkey,
      ];
      if (specs.some((s) => s && (isModsSpec(s) || isTap2Spec(s)))) return true;
    }
    return false;
  });

  onMount(() => {
    void refreshDiagnostics();
    void refreshAx();
    // Backend re-registers hotkeys on every save; refetch so the
    // picker reflects the OS truth (including fallback / "Not bound").
    void (async () => {
      unlistenSettings = await onSettingsChanged(() => {
        void refreshDiagnostics();
      });
    })();
  });

  // Per design spec §7: no spinner; on grant, the panel's slot 3
  // collapses and Save enables. The poll only ticks while a row
  // actually needs the answer — see `pollActive` above.
  createEffect(() => {
    if (pollActive()) {
      if (axPoll == null) {
        axPoll = setInterval(() => {
          void refreshAx();
        }, 1500);
      }
    } else if (axPoll != null) {
      clearInterval(axPoll);
      axPoll = null;
    }
  });

  onCleanup(() => {
    unlistenSettings?.();
    if (axPoll != null) clearInterval(axPoll);
  });

  async function onTest() {
    try {
      const a = await getActiveHotkey();
      if (!a.bound) {
        setTestStatus(`Failed: ${a.error ?? "no fallback succeeded"}`);
      } else if (a.fell_back) {
        setTestStatus(
          `Bound to ${formatHotkey(a.bound)} (fallback from ${formatHotkey(a.configured)})`,
        );
      } else {
        setTestStatus(`Bound to ${formatHotkey(a.bound)}`);
      }
    } catch (e) {
      setTestStatus(`Failed: ${String(e)}`);
    }
  }

  const ptt = () => diags().get("push-to-talk") ?? null;
  const cancel = () => diags().get("cancel") ?? null;
  const cycle = () => diags().get("mode-cycle") ?? null;

  // Snapshot the other two rows so a captured spec can be validated
  // against them without leaning on stale closure state.
  const otherSpecs = (self: HotkeyKind): { kind: HotkeyKind; label: string; spec: string }[] => {
    const rows: { kind: HotkeyKind; label: string; spec: string }[] = [
      { kind: "ptt", label: "Push-to-talk", spec: props.s.hotkey },
      { kind: "cancel", label: "Cancel", spec: props.s.cancel_hotkey },
      { kind: "cycle", label: "Cycle voice mode", spec: props.s.mode_cycle_hotkey },
    ];
    return rows.filter((r) => r.kind !== self && r.spec !== "");
  };

  function patchFor(kind: HotkeyKind, value: string): Partial<Settings> {
    if (kind === "ptt") return { hotkey: value };
    if (kind === "cancel") return { cancel_hotkey: value };
    return { mode_cycle_hotkey: value };
  }
  // Resolution "Use this and clear that one" path. PTT has no
  // off-state — fall back to its default so the user is never left
  // unable to dictate.
  function clearValue(kind: HotkeyKind): string {
    return kind === "ptt" ? DEFAULT_PUSH_TO_TALK : "";
  }

  return (
    <>
      <SectionHeader title="Primary shortcut" />
      <Card>
        <HotkeyRow
          kind="ptt"
          label="Push-to-talk"
          hint={pttHint(props.s.hands_free_enabled, ptt())}
          configured={props.s.hotkey}
          defaultSpec={DEFAULT_PUSH_TO_TALK}
          allowEmpty={false}
          diagnostic={ptt()}
          others={otherSpecs("ptt")}
          axGranted={axGranted()}
          autoOpen={autoCapture() === "ptt"}
          onAutoOpened={() => setAutoCapture(null)}
          flash={flashRow() === "ptt"}
          onChange={(v) => props.update({ hotkey: v })}
          onResolveConflict={(self, other) => {
            props.update({
              ...patchFor("ptt", self),
              ...patchFor(other, clearValue(other)),
            });
            triggerFlash(other);
          }}
          onChangeOther={(k) => setAutoCapture(k)}
          onCapturingAxSpecChange={(a) => setRowAx("ptt", a)}
          extra={
            <div class="flex flex-col items-end gap-1">
              <button
                type="button"
                onClick={() => void onTest()}
                class="rounded-[6px] bg-[var(--color-control)] px-3 py-1 text-[11px] hover:bg-[var(--color-control-hover)]"
                title="Read the current binding from the runtime"
              >
                Test binding
              </button>
              <Show when={testStatus()}>
                <span
                  class="text-[10px]"
                  classList={{
                    "text-[var(--color-success)]":
                      testStatus()?.startsWith("Bound") ?? false,
                    "text-[var(--color-danger)]":
                      testStatus()?.startsWith("Failed") ?? false,
                  }}
                >
                  {testStatus()}
                </span>
              </Show>
            </div>
          }
          last
        />
      </Card>

      <SectionHeader title="Secondary shortcuts" />
      <Card>
        <HotkeyRow
          kind="cancel"
          label="Cancel while recording"
          hint="Abort the pipeline without copying."
          configured={props.s.cancel_hotkey}
          defaultSpec={DEFAULT_CANCEL_HOTKEY}
          allowEmpty
          diagnostic={cancel()}
          others={otherSpecs("cancel")}
          axGranted={axGranted()}
          autoOpen={autoCapture() === "cancel"}
          onAutoOpened={() => setAutoCapture(null)}
          flash={flashRow() === "cancel"}
          onChange={(v) => props.update({ cancel_hotkey: v })}
          onResolveConflict={(self, other) => {
            props.update({
              ...patchFor("cancel", self),
              ...patchFor(other, clearValue(other)),
            });
            triggerFlash(other);
          }}
          onChangeOther={(k) => setAutoCapture(k)}
          onCapturingAxSpecChange={(a) => setRowAx("cancel", a)}
        />
        <HotkeyRow
          kind="cycle"
          label="Cycle voice mode"
          hint="Press to advance through Voice Modes in display order."
          configured={props.s.mode_cycle_hotkey}
          defaultSpec={DEFAULT_MODE_CYCLE_HOTKEY}
          allowEmpty
          diagnostic={cycle()}
          others={otherSpecs("cycle")}
          axGranted={axGranted()}
          autoOpen={autoCapture() === "cycle"}
          onAutoOpened={() => setAutoCapture(null)}
          flash={flashRow() === "cycle"}
          onChange={(v) => props.update({ mode_cycle_hotkey: v })}
          onResolveConflict={(self, other) => {
            props.update({
              ...patchFor("cycle", self),
              ...patchFor(other, clearValue(other)),
            });
            triggerFlash(other);
          }}
          onChangeOther={(k) => setAutoCapture(k)}
          onCapturingAxSpecChange={(a) => setRowAx("cycle", a)}
          last
        />
      </Card>
      <p class="text-[11px] text-[var(--color-ink-mute)]">
        Changes apply immediately. If the configured key fails to bind on
        macOS, Vox falls back to the next compatible shortcut and shows
        it under the picker.
      </p>
    </>
  );
}
