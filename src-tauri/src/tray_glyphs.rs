// Procedurally-drawn glyphs for the menu-bar tray icon.
//
// Vocabulary follows DESIGN.md §5.10 / §7 and the competitor research:
// tray glyphs are *functional state indicators*, not brand mimics. Four of
// the five states render as alpha-only template images that AppKit tints to
// match the menu bar (white in dark mode, near-black in light mode). The
// one exception is `recording`, where we break template to fill a solid
// coral disc — the brand colour earning its keep at the one moment it
// belongs (see §5.4 of DESIGN.md). The system's own orange privacy dot
// handles the "your mic is hot" message, so Vox's coral is purely a brand
// cue and isn't competing with the OS for attention.
//
// Animated states (Recording, Processing) take a `phase: f32` in `[0, 1)`
// that the caller advances over time. Stateless rendering keeps the
// drawing functions trivially testable and lets the frame timer live in
// `tray_icon.rs` where it can pick rate per-state.
//
// This module is `include!`d by `build.rs` to dump designer-reference PNGs,
// so it must not depend on Tauri/AppKit symbols — only `std`.

use std::f32::consts::{FRAC_PI_4, PI, TAU};

// ---- coordinate helpers ------------------------------------------------

fn idx(x: i32, y: i32, w: i32) -> Option<usize> {
    if x < 0 || y < 0 || x >= w || y >= w {
        None
    } else {
        Some(((y * w + x) * 4) as usize)
    }
}

/// Template-image write: only alpha matters; RGB stays 0 so AppKit can
/// tint freely. Multiple strokes overlap via max-of-two so they don't
/// darken (alpha isn't additive in template land).
fn put(rgba: &mut [u8], x: i32, y: i32, w: i32, alpha: u8) {
    if let Some(i) = idx(x, y, w) {
        let cur = rgba[i + 3];
        rgba[i + 3] = cur.max(alpha);
    }
}

/// Coloured-image write: source-over alpha compositing onto whatever was
/// there before. Used only by `recording()`, which is shipped as a
/// non-template image.
fn blend_rgba(rgba: &mut [u8], x: i32, y: i32, w: i32, src: [u8; 4]) {
    let Some(i) = idx(x, y, w) else { return };
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = rgba[i + 3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    for c in 0..3 {
        let s = src[c] as f32 / 255.0;
        let d = rgba[i + c] as f32 / 255.0;
        let o = (s * sa + d * da * (1.0 - sa)) / out_a;
        rgba[i + c] = (o * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    rgba[i + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn rect(rgba: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, w: i32, alpha: u8) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            put(rgba, x, y, w, alpha);
        }
    }
}

/// Anti-aliased filled circle, alpha-only (template).
fn circle(rgba: &mut [u8], cx: f32, cy: f32, r: f32, w: i32, alpha: u8) {
    let r2 = r * r;
    let xmin = (cx - r - 1.0).floor() as i32;
    let xmax = (cx + r + 1.0).ceil() as i32;
    let ymin = (cy - r - 1.0).floor() as i32;
    let ymax = (cy + r + 1.0).ceil() as i32;
    for y in ymin..=ymax {
        for x in xmin..=xmax {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2 {
                put(rgba, x, y, w, alpha);
            } else if d2 <= (r + 1.0) * (r + 1.0) {
                let dist = d2.sqrt() - r;
                let a = ((1.0 - dist).clamp(0.0, 1.0) * alpha as f32) as u8;
                put(rgba, x, y, w, a);
            }
        }
    }
}

/// Anti-aliased filled circle, RGBA (coloured, source-over).
fn circle_rgba(rgba: &mut [u8], cx: f32, cy: f32, r: f32, w: i32, color: [u8; 4]) {
    let r2 = r * r;
    let xmin = (cx - r - 1.0).floor() as i32;
    let xmax = (cx + r + 1.0).ceil() as i32;
    let ymin = (cy - r - 1.0).floor() as i32;
    let ymax = (cy + r + 1.0).ceil() as i32;
    for y in ymin..=ymax {
        for x in xmin..=xmax {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2 {
                blend_rgba(rgba, x, y, w, color);
            } else if d2 <= (r + 1.0) * (r + 1.0) {
                let dist = d2.sqrt() - r;
                let edge = ((1.0 - dist).clamp(0.0, 1.0) * color[3] as f32) as u8;
                blend_rgba(rgba, x, y, w, [color[0], color[1], color[2], edge]);
            }
        }
    }
}

/// Anti-aliased ring (hollow circle), alpha-only.
fn ring(rgba: &mut [u8], cx: f32, cy: f32, r: f32, stroke: f32, w: i32, alpha: u8) {
    let r_out = r;
    let r_in = (r - stroke).max(0.0);
    let xmin = (cx - r_out - 1.0).floor() as i32;
    let xmax = (cx + r_out + 1.0).ceil() as i32;
    let ymin = (cy - r_out - 1.0).floor() as i32;
    let ymax = (cy + r_out + 1.0).ceil() as i32;
    for y in ymin..=ymax {
        for x in xmin..=xmax {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d <= r_out && d >= r_in {
                put(rgba, x, y, w, alpha);
            } else if d <= r_out + 1.0 && d >= r_in - 1.0 {
                let edge_dist = if d > r_out {
                    d - r_out
                } else if d < r_in {
                    r_in - d
                } else {
                    0.0
                };
                let a = ((1.0 - edge_dist).clamp(0.0, 1.0) * alpha as f32) as u8;
                put(rgba, x, y, w, a);
            }
        }
    }
}

/// Anti-aliased ring (hollow circle), RGBA (coloured, source-over). Used
/// by the recording-state breathing halo.
fn ring_rgba(rgba: &mut [u8], cx: f32, cy: f32, r: f32, stroke: f32, w: i32, color: [u8; 4]) {
    let r_out = r;
    let r_in = (r - stroke).max(0.0);
    let xmin = (cx - r_out - 1.0).floor() as i32;
    let xmax = (cx + r_out + 1.0).ceil() as i32;
    let ymin = (cy - r_out - 1.0).floor() as i32;
    let ymax = (cy + r_out + 1.0).ceil() as i32;
    for y in ymin..=ymax {
        for x in xmin..=xmax {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d <= r_out && d >= r_in {
                blend_rgba(rgba, x, y, w, color);
            } else if d <= r_out + 1.0 && d >= r_in - 1.0 {
                let edge_dist = if d > r_out {
                    d - r_out
                } else if d < r_in {
                    r_in - d
                } else {
                    0.0
                };
                let edge = ((1.0 - edge_dist).clamp(0.0, 1.0) * color[3] as f32) as u8;
                blend_rgba(rgba, x, y, w, [color[0], color[1], color[2], edge]);
            }
        }
    }
}

// ---- shape primitives --------------------------------------------------

/// Hollow microphone — capsule body on a base arc with a short stand.
/// Used for `Idle` and `Error` (with an overlaid bang badge).
fn mic_outline(rgba: &mut [u8], w: i32) {
    let s = w as f32 / 16.0;
    let cx = w as f32 / 2.0;
    let cy = w as f32 / 2.0 - 1.5 * s;

    let body_w = 6.0 * s;
    let body_h = 9.0 * s;
    let body_top = (cy - body_h / 2.0).round() as i32;
    let body_bot = (cy + body_h / 2.0).round() as i32;
    let body_left = (cx - body_w / 2.0).round() as i32;
    let body_right = (cx + body_w / 2.0).round() as i32;
    let radius = body_w / 2.0;

    // Capsule: two vertical strokes + two end-cap rings.
    let stroke = (1.5 * s).max(1.0);
    for y in body_top..=body_bot {
        for d in 0..stroke.ceil() as i32 {
            put(rgba, body_left + d, y, w, 220);
            put(rgba, body_right - d, y, w, 220);
        }
    }
    ring(rgba, cx, cy - body_h / 2.0 + radius, radius, stroke, w, 220);
    ring(rgba, cx, cy + body_h / 2.0 - radius, radius, stroke, w, 220);

    // Base arc (the swoosh under the mic body).
    let arc_r = (5.0 * s).round();
    let arc_cy = cy + body_h / 2.0 + 0.5 * s;
    let r2_out = arc_r * arc_r;
    let r2_in = (arc_r - stroke) * (arc_r - stroke);
    for y in (arc_cy.round() as i32)..((arc_cy + arc_r + 1.0).ceil() as i32) {
        for x in ((cx - arc_r - 1.0).floor() as i32)..=((cx + arc_r + 1.0).ceil() as i32) {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - arc_cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2_out && d2 >= r2_in {
                put(rgba, x, y, w, 220);
            }
        }
    }

    // Stand stem.
    let stem_top = (arc_cy + arc_r - 1.0 * s).round() as i32;
    let stem_bot = (arc_cy + arc_r + 1.5 * s).round() as i32;
    let stem_left = (cx - 0.5 * s).round() as i32;
    let stem_right = (cx + 0.5 * s).round() as i32;
    rect(rgba, stem_left, stem_top, stem_right, stem_bot, w, 220);

    // Stand foot.
    let foot_left = (cx - 1.5 * s).round() as i32;
    let foot_right = (cx + 1.5 * s).round() as i32;
    rect(rgba, foot_left, stem_bot, foot_right, stem_bot + (0.75 * s).ceil() as i32, w, 220);
}

/// Padlock — body rect + shackle arc on top. Used for `Permission`.
fn lock_glyph(rgba: &mut [u8], w: i32) {
    let s = w as f32 / 16.0;
    let cx = w as f32 / 2.0;
    let cy = w as f32 / 2.0 + 1.0 * s;

    let body_top = (cy - 2.5 * s).round() as i32;
    let body_bot = (cy + 3.5 * s).round() as i32;
    let body_left = (cx - 3.5 * s).round() as i32;
    let body_right = (cx + 3.5 * s).round() as i32;
    rect(rgba, body_left, body_top, body_right, body_bot, w, 255);

    let shackle_r = 2.5 * s;
    let shackle_cy = cy - 2.5 * s;
    let stroke = (1.5 * s).max(1.0);
    let r2_out = shackle_r * shackle_r;
    let r2_in = (shackle_r - stroke) * (shackle_r - stroke);
    for y in 0..w {
        for x in 0..w {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - shackle_cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2_out && d2 >= r2_in && dy <= 0.5 * s {
                put(rgba, x, y, w, 255);
            }
        }
    }
}

/// Small bang badge in the lower-right corner — used by `Error`. Sized
/// down from the old centred bang so it reads as a notification overlay
/// on the mic rather than replacing it.
fn bang_badge(rgba: &mut [u8], w: i32) {
    let s = w as f32 / 16.0;
    let bcx = w as f32 - 4.0 * s;
    let bcy = w as f32 - 4.0 * s;

    let bar_w = (1.5 * s).max(1.0);
    let bar_top = (bcy - 2.0 * s).round() as i32;
    let bar_bot = (bcy + 0.5 * s).round() as i32;
    let bar_left = (bcx - bar_w / 2.0).round() as i32;
    let bar_right = (bcx + bar_w / 2.0).round() as i32;
    rect(rgba, bar_left, bar_top, bar_right, bar_bot, w, 255);
    circle(rgba, bcx, bcy + 1.75 * s, 0.7 * s, w, 255);
}

// ---- public per-state renderers ---------------------------------------

fn canvas(w: i32) -> Vec<u8> {
    vec![0u8; (w * w * 4) as usize]
}

/// Calm hollow microphone — the default "Vox is here, ready" state.
pub fn idle(w: i32) -> Vec<u8> {
    let mut rgba = canvas(w);
    mic_outline(&mut rgba, w);
    rgba
}

/// Solid coral disc with a sine-eased breathing pulse. Caller passes
/// `phase ∈ [0, 1)` representing the position in a 1 Hz cycle.
///
/// **Non-template**: this is the one state that ships in colour. The dot
/// alpha breathes 0.55 ↔ 1.0 and a faint outer halo expands from the dot
/// edge then dissipates, like a sound wave. The size of the bounding box
/// stays constant so the menu-bar hit target doesn't shift.
pub fn recording(w: i32, phase: f32) -> Vec<u8> {
    let mut rgba = canvas(w);
    let s = w as f32 / 16.0;
    let cx = w as f32 / 2.0;
    let cy = w as f32 / 2.0;

    // Sine breath in [0, 1]: starts at 0.5, peaks at 1.0, troughs at 0.0.
    let breath = (phase * TAU).sin() * 0.5 + 0.5;

    // Halo: starts at the dot edge, expands outward, fades to zero. Tuned
    // to feel like a single ripple per cycle rather than a constant glow.
    let halo_r = (4.0 + 1.6 * breath) * s;
    let halo_alpha = ((1.0 - breath) * 0.55 * 255.0) as u8;
    if halo_alpha > 0 {
        ring_rgba(
            &mut rgba,
            cx,
            cy,
            halo_r,
            (1.0 * s).max(1.0),
            w,
            [255, 154, 124, halo_alpha],
        );
    }

    // Main coral dot. Keep the alpha range narrow (0.55..1.0) so it never
    // disappears — the user should always see *something* while recording.
    let dot_a = (0.55 + 0.45 * breath) as f32;
    let dot_r = 4.0 * s;
    circle_rgba(
        &mut rgba,
        cx,
        cy,
        dot_r,
        w,
        [255, 122, 92, (dot_a * 255.0) as u8],
    );

    rgba
}

/// Three-quarter ring with a rotating gap. `phase ∈ [0, 1)` advances the
/// gap clockwise around the circle. Template (alpha-only).
pub fn processing(w: i32, phase: f32) -> Vec<u8> {
    let mut rgba = canvas(w);
    let s = w as f32 / 16.0;
    let cx = w as f32 / 2.0;
    let cy = w as f32 / 2.0;
    let r = 5.5 * s;
    let stroke = (1.5 * s).max(1.0);

    // Gap is 90° wide and rotates clockwise. atan2 returns (-PI, PI]
    // with +x = 0 and +y = +PI/2; screen y grows downward, so positive
    // phase advances the gap clockwise on screen.
    let gap_center = phase * TAU - PI;
    let gap_half = FRAC_PI_4;

    let r_in = (r - stroke).max(0.0);
    let xmin = (cx - r - 1.0).floor() as i32;
    let xmax = (cx + r + 1.0).ceil() as i32;
    let ymin = (cy - r - 1.0).floor() as i32;
    let ymax = (cy + r + 1.0).ceil() as i32;
    for y in ymin..=ymax {
        for x in xmin..=xmax {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();

            // Outside the ring (with a 1 px AA halo on each side)? Skip.
            if d > r + 1.0 || d < r_in - 1.0 {
                continue;
            }

            // Inside the gap wedge? Skip.
            let mut delta = (dy.atan2(dx) - gap_center).abs();
            if delta > PI {
                delta = TAU - delta;
            }
            if delta < gap_half {
                continue;
            }

            let a = if d <= r && d >= r_in {
                220
            } else {
                let edge_dist = if d > r { d - r } else { r_in - d };
                ((1.0 - edge_dist).clamp(0.0, 1.0) * 220.0) as u8
            };
            put(&mut rgba, x, y, w, a);
        }
    }
    rgba
}

/// Hollow microphone with a small bang badge in the lower-right corner.
/// Static — DESIGN.md and the research both agree errors are *blockers*,
/// not events, and shouldn't pulse for attention.
pub fn error(w: i32) -> Vec<u8> {
    let mut rgba = canvas(w);
    mic_outline(&mut rgba, w);
    bang_badge(&mut rgba, w);
    rgba
}

/// Padlock glyph — needs Mic / Accessibility permission. Static.
pub fn permission(w: i32) -> Vec<u8> {
    let mut rgba = canvas(w);
    lock_glyph(&mut rgba, w);
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nonzero_alpha_count(rgba: &[u8]) -> usize {
        rgba.chunks_exact(4).filter(|p| p[3] > 0).count()
    }

    #[test]
    fn idle_renders_pixels() {
        let buf = idle(32);
        assert_eq!(buf.len(), 32 * 32 * 4);
        assert!(nonzero_alpha_count(&buf) > 50, "idle mic should draw visible pixels");
    }

    #[test]
    fn recording_uses_coral_rgb() {
        // Pick a phase where the dot is at full alpha to make assertion easy.
        let buf = recording(32, 0.25);
        let centre_idx = (16 * 32 + 16) * 4;
        let r = buf[centre_idx];
        let g = buf[centre_idx + 1];
        let b = buf[centre_idx + 2];
        let a = buf[centre_idx + 3];
        assert!(a > 200, "centre of recording dot should be near-opaque, got {a}");
        // Coral is warm — red dominates, green > blue, both well below red.
        assert!(r > 200, "coral red channel: {r}");
        assert!(g > 80 && g < 180, "coral green channel: {g}");
        assert!(b > 50 && b < 140, "coral blue channel: {b}");
    }

    #[test]
    fn recording_phases_differ() {
        // Phase 0.25 is the breath peak (sin(π/2) = 1), phase 0.0 is the
        // midpoint (sin(0) = 0). They must produce different buffers,
        // otherwise the animation is invisible. Note: 0.0 and 0.5 would
        // be identical here because of sine symmetry.
        let a = recording(32, 0.0);
        let b = recording(32, 0.25);
        assert_ne!(a, b, "recording should change with phase");
    }

    #[test]
    fn processing_phases_differ() {
        let a = processing(32, 0.0);
        let b = processing(32, 0.5);
        assert_ne!(a, b, "processing gap should rotate with phase");
    }

    #[test]
    fn processing_is_template() {
        // All RGB should be 0 — template image, AppKit tints.
        let buf = processing(32, 0.3);
        for p in buf.chunks_exact(4) {
            assert_eq!((p[0], p[1], p[2]), (0, 0, 0), "processing must be alpha-only");
        }
    }

    #[test]
    fn error_renders_pixels() {
        let buf = error(32);
        assert!(nonzero_alpha_count(&buf) > 50, "error glyph should draw visible pixels");
    }

    #[test]
    fn permission_renders_pixels() {
        let buf = permission(32);
        assert!(nonzero_alpha_count(&buf) > 30, "permission lock should draw visible pixels");
    }

    #[test]
    fn template_glyphs_are_alpha_only() {
        for buf in [idle(32), error(32), permission(32)] {
            for p in buf.chunks_exact(4) {
                assert_eq!((p[0], p[1], p[2]), (0, 0, 0));
            }
        }
    }
}
