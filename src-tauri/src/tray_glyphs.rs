//! Procedurally-drawn template glyphs for the menu-bar tray icon.
//!
//! Output is RGBA where R/G/B are zero and only the alpha channel is
//! populated. This is what macOS expects for a "template image" — the
//! system tints the alpha to match the menu-bar color (white in dark
//! mode, near-black in light mode). The same generator is used at build
//! time (build.rs writes PNGs to `icons/tray/` for design review) and
//! at runtime (this module renders directly into a Tauri `Image`).

fn idx(x: i32, y: i32, w: i32) -> Option<usize> {
    if x < 0 || y < 0 || x >= w || y >= w {
        None
    } else {
        Some(((y * w + x) * 4) as usize)
    }
}

fn put(rgba: &mut [u8], x: i32, y: i32, w: i32, alpha: u8) {
    if let Some(i) = idx(x, y, w) {
        let cur = rgba[i + 3];
        rgba[i + 3] = cur.max(alpha);
    }
}

fn rect(rgba: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, w: i32, alpha: u8) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            put(rgba, x, y, w, alpha);
        }
    }
}

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

fn mic(rgba: &mut [u8], w: i32, filled: bool) {
    let s = w as f32 / 16.0;

    let body_w = 6.0 * s;
    let body_h = 9.0 * s;
    let cx = w as f32 / 2.0;
    let cy = w as f32 / 2.0 - 1.5 * s;

    let body_top = (cy - body_h / 2.0).round() as i32;
    let body_bot = (cy + body_h / 2.0).round() as i32;
    let body_left = (cx - body_w / 2.0).round() as i32;
    let body_right = (cx + body_w / 2.0).round() as i32;
    let radius = body_w / 2.0;

    if filled {
        rect(
            rgba,
            body_left,
            (cy - body_h / 2.0 + radius).round() as i32,
            body_right,
            (cy + body_h / 2.0 - radius).round() as i32,
            w,
            255,
        );
        circle(rgba, cx, cy - body_h / 2.0 + radius, radius, w, 255);
        circle(rgba, cx, cy + body_h / 2.0 - radius, radius, w, 255);
    } else {
        let stroke = (1.25 * s).max(1.0);
        for y in body_top..=body_bot {
            for d in 0..stroke.ceil() as i32 {
                put(rgba, body_left + d, y, w, 220);
                put(rgba, body_right - d, y, w, 220);
            }
        }
        ring(rgba, cx, cy - body_h / 2.0 + radius, radius, stroke, w, 220);
        ring(rgba, cx, cy + body_h / 2.0 - radius, radius, stroke, w, 220);
    }

    let arc_r = (5.0 * s).round();
    let arc_cx = cx;
    let arc_cy = cy + body_h / 2.0 + 0.5 * s;
    let stroke = (1.25 * s).max(1.0);
    let r2_out = arc_r * arc_r;
    let r2_in = (arc_r - stroke) * (arc_r - stroke);
    for y in (arc_cy.round() as i32)..((arc_cy + arc_r + 1.0).ceil() as i32) {
        for x in ((arc_cx - arc_r - 1.0).floor() as i32)..=((arc_cx + arc_r + 1.0).ceil() as i32) {
            let dx = x as f32 + 0.5 - arc_cx;
            let dy = y as f32 + 0.5 - arc_cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2_out && d2 >= r2_in {
                put(rgba, x, y, w, 220);
            }
        }
    }

    let stem_top = (arc_cy + arc_r - 1.0 * s).round() as i32;
    let stem_bot = (arc_cy + arc_r + 1.5 * s).round() as i32;
    let stem_left = (cx - 0.5 * s).round() as i32;
    let stem_right = (cx + 0.5 * s).round() as i32;
    rect(rgba, stem_left, stem_top, stem_right, stem_bot, w, 220);

    let foot_left = (cx - 1.5 * s).round() as i32;
    let foot_right = (cx + 1.5 * s).round() as i32;
    let foot_y_top = stem_bot;
    let foot_y_bot = stem_bot + (0.75 * s).ceil() as i32;
    rect(rgba, foot_left, foot_y_top, foot_right, foot_y_bot, w, 220);
}

fn rec_dot(rgba: &mut [u8], w: i32) {
    let s = w as f32 / 16.0;
    let r = 2.5 * s;
    let cx = w as f32 - r - 0.5 * s;
    let cy = w as f32 - r - 0.5 * s;
    circle(rgba, cx, cy, r, w, 255);
}

fn spinner(rgba: &mut [u8], w: i32) {
    let s = w as f32 / 16.0;
    let cx = w as f32 / 2.0;
    let cy = w as f32 / 2.0;
    let r = 5.5 * s;
    let stroke = (1.5 * s).max(1.0);
    let r2_out = r * r;
    let r2_in = (r - stroke) * (r - stroke);
    for y in 0..w {
        for x in 0..w {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 > r2_out || d2 < r2_in {
                continue;
            }
            if dx > 0.0 && dy > 0.0 {
                continue;
            }
            put(rgba, x, y, w, 220);
        }
    }
}

fn bang(rgba: &mut [u8], w: i32) {
    let s = w as f32 / 16.0;
    let cx = w as f32 / 2.0;
    let bar_w = (1.5 * s).max(1.0);
    let bar_top = (3.0 * s).round() as i32;
    let bar_bot = (10.0 * s).round() as i32;
    let bar_left = (cx - bar_w / 2.0).round() as i32;
    let bar_right = (cx + bar_w / 2.0).round() as i32;
    rect(rgba, bar_left, bar_top, bar_right, bar_bot, w, 255);
    circle(rgba, cx, 12.5 * s, 1.0 * s, w, 255);
}

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
    let shackle_cx = cx;
    let shackle_cy = cy - 2.5 * s;
    let stroke = (1.5 * s).max(1.0);
    let r2_out = shackle_r * shackle_r;
    let r2_in = (shackle_r - stroke) * (shackle_r - stroke);
    for y in 0..w {
        for x in 0..w {
            let dx = x as f32 + 0.5 - shackle_cx;
            let dy = y as f32 + 0.5 - shackle_cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2_out && d2 >= r2_in && dy <= 0.5 * s {
                put(rgba, x, y, w, 255);
            }
        }
    }
}

// ---- public draw API -------------------------------------------------------

pub fn idle(w: i32) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * w * 4) as usize];
    mic(&mut rgba, w, false);
    rgba
}

pub fn recording(w: i32, with_dot: bool) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * w * 4) as usize];
    mic(&mut rgba, w, true);
    if with_dot {
        rec_dot(&mut rgba, w);
    }
    rgba
}

pub fn processing(w: i32) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * w * 4) as usize];
    spinner(&mut rgba, w);
    rgba
}

pub fn error(w: i32) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * w * 4) as usize];
    mic(&mut rgba, w, false);
    bang(&mut rgba, w);
    rgba
}

pub fn permission(w: i32) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * w * 4) as usize];
    lock_glyph(&mut rgba, w);
    rgba
}
