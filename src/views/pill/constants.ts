/// Mode-cycle HUD lifetime for the full-takeover path (idle mode-cycle).
export const MODE_HUD_MS = 800;
/// Companion chip lifetime when mode fires during an active recording
/// session — long enough to read comfortably while listening starts.
export const MODE_CHIP_MS = 2000;

export const BARS = 32;

/** Soft cap on how many characters per second the streaming UI
 * reveals. Tuned to slightly LAG Gemma's ~50 tok/s × ~4 char/tok =
 * ~200 cps generation rate so visible text doesn't snap per token
 * but doesn't fall multiple sentences behind either. 180 cps lets a
 * 200-char polish that streams in 1 s reveal in ~1.1 s — enough lead
 * for the eye to track without a jarring snap on `final: true`. */
export const REVEAL_CHARS_PER_SEC = 180;
export const REVEAL_TICK_MS = 33; // ~30 Hz

/// Length of the post-final transcript hold. Picked so the user has
/// a beat to read the final text before the polishing stage takes
/// the slot — short enough to not feel like the app is hung when
/// polish is fast.
export const TRANSCRIPT_HOLD_MS = 2000;

// ─── Dynamic width-floor lerp ──────────────────────────────────────
// Couples width to height so a tall, narrow pill never happens. As
// the streaming content grows in height, the pill's *minimum* width
// ramps up via an ease-out curve, forcing the box to widen rather
// than balloon vertically. `fit-content` still allows the natural
// width to exceed the floor when content needs more.
//
// H_min/H_max bracket the ramp; H_max should track HEIGHT_MAX in
// pill.rs (120). W_min is 180 px — single-line streaming feels
// intentional at that width. W_max is 360 px.
export const H_MIN = 32;
export const H_MAX = 120;
export const W_MIN = 180;
export const W_MAX = 360;
// 12 px quantisation grid + 12 px grow / 32 px shrink hysteresis
// bands stop the floor from chasing every sub-pixel content nudge
// (each one would otherwise round-trip to Rust as a window resize).
export const FLOOR_GRID = 12;
export const FLOOR_GROW_TRIGGER = 12;
export const FLOOR_SHRINK_TRIGGER = 32;
// Sustain time before a shrink commits. Measured in ms via
// performance.now() at the rAF callback; a sub-32 px content
// height drop has to persist this long before the floor steps down.
export const FLOOR_SHRINK_SUSTAIN_MS = 250;

// Window outer frame is 16 px wider and 10 px taller than the
// visible pill so the OS drop shadow + 1 px rim have room to fall
// off. Mirror that here when reporting back so Rust resizes to the
// right outer dimensions.
export const WINDOW_PADDING_W = 16;
export const WINDOW_PADDING_H = 10;
