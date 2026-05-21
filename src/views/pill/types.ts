export type PillState =
  | "listening"
  | "transcribing"
  | "polishing"
  | "done"
  | "error"
  | "canceled"
  | "saturated";

export interface PillStatePayload {
  state: PillState;
  message?: string;
  /** Soft-fail flag on `Done` — clipboard succeeded but a step
   *  (typically LLM polish) was substituted. Drives the amber
   *  warning treatment so the user notices their cleanup was
   *  skipped instead of mistaking it for a successful polish. */
  warning?: boolean;
  /** Count of background (non-foreground) in-flight sessions.
   *  Only present when non-zero — the field is omitted from the
   *  Rust payload via `skip_serializing_if`. */
  background_count?: number;
}

export interface AudioLevelPayload {
  level: number;
}

export interface ToastPayload {
  message: string;
  ms: number;
}

export interface StreamPayload {
  session_id: number;
  text: string;
  final: boolean;
}

export interface ActiveModePayload {
  id: string;
  name: string;
  icon: string;
}
