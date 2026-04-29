use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod tray_glyphs {
    // ---- coordinate helpers ------------------------------------------------

    fn idx(x: i32, y: i32, w: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= w || y >= w {
            None
        } else {
            Some(((y * w + x) * 4) as usize)
        }
    }

    fn put(rgba: &mut [u8], x: i32, y: i32, w: i32, alpha: u8) {
        if let Some(i) = idx(x, y, w) {
            // Template image: only alpha varies; RGB stays 0 so macOS can
            // tint freely. We additively blend alpha (max-of-two) so multiple
            // strokes overlap without darkening.
            let cur = rgba[i + 3];
            rgba[i + 3] = cur.max(alpha);
        }
    }

    /// Draws a filled (or outlined) rounded rect.
    fn rect(rgba: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, w: i32, alpha: u8) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                put(rgba, x, y, w, alpha);
            }
        }
    }

    /// Filled circle at (cx, cy) with radius r.
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
                    // Soft edge — anti-alias.
                    let dist = d2.sqrt() - r;
                    let a = ((1.0 - dist).clamp(0.0, 1.0) * alpha as f32) as u8;
                    put(rgba, x, y, w, a);
                }
            }
        }
    }

    /// Hollow circle (1 px stroke at the given scale).
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

    /// Mic glyph: rounded body + base bar + stem. Centered for the given
    /// canvas size. `filled` controls whether the body is filled (recording)
    /// or just outlined (idle).
    fn mic(rgba: &mut [u8], w: i32, filled: bool) {
        let s = w as f32 / 16.0; // unit scale: 1.0 at 16px, 2.0 at 32px

        // Body dims.
        let body_w = 6.0 * s;
        let body_h = 9.0 * s;
        let cx = w as f32 / 2.0;
        let cy = w as f32 / 2.0 - 1.5 * s;

        // Filled rounded rect — approximated by a rect + two end-caps
        // (semicircles).
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
            // Top + bottom rounded caps.
            circle(rgba, cx, cy - body_h / 2.0 + radius, radius, w, 255);
            circle(rgba, cx, cy + body_h / 2.0 - radius, radius, w, 255);
        } else {
            // 1 px stroke (well, ~1.25 px scaled).
            let stroke = (1.25 * s).max(1.0);
            // Vertical sides.
            for y in body_top..=body_bot {
                for d in 0..stroke.ceil() as i32 {
                    put(rgba, body_left + d, y, w, 220);
                    put(rgba, body_right - d, y, w, 220);
                }
            }
            // Top + bottom caps.
            ring(rgba, cx, cy - body_h / 2.0 + radius, radius, stroke, w, 220);
            ring(rgba, cx, cy + body_h / 2.0 - radius, radius, stroke, w, 220);
        }

        // Base arc (the swoosh under the mic body).
        let arc_r = (5.0 * s).round();
        let arc_cx = cx;
        let arc_cy = cy + body_h / 2.0 + 0.5 * s;
        let stroke = (1.25 * s).max(1.0);
        let r2_out = arc_r * arc_r;
        let r2_in = (arc_r - stroke) * (arc_r - stroke);
        for y in (arc_cy.round() as i32)..((arc_cy + arc_r + 1.0).ceil() as i32) {
            for x in ((arc_cx - arc_r - 1.0).floor() as i32)..=((arc_cx + arc_r + 1.0).ceil() as i32)
            {
                let dx = x as f32 + 0.5 - arc_cx;
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
        let foot_y_top = stem_bot;
        let foot_y_bot = stem_bot + (0.75 * s).ceil() as i32;
        rect(rgba, foot_left, foot_y_top, foot_right, foot_y_bot, w, 220);
    }

    /// Recording dot — solid red dot bottom-right (template alpha only;
    /// AppKit will tint accent or red as configured).
    fn rec_dot(rgba: &mut [u8], w: i32) {
        let s = w as f32 / 16.0;
        let r = 2.5 * s;
        let cx = w as f32 - r - 0.5 * s;
        let cy = w as f32 - r - 0.5 * s;
        circle(rgba, cx, cy, r, w, 255);
    }

    /// Spinner arc (3/4 circle) — used for processing.
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
                // Skip the bottom-right quadrant for a "loading" look.
                if dx > 0.0 && dy > 0.0 {
                    continue;
                }
                put(rgba, x, y, w, 220);
            }
        }
    }

    /// Exclamation mark — used for error.
    fn bang(rgba: &mut [u8], w: i32) {
        let s = w as f32 / 16.0;
        let cx = w as f32 / 2.0;
        // Vertical bar.
        let bar_w = (1.5 * s).max(1.0);
        let bar_top = (3.0 * s).round() as i32;
        let bar_bot = (10.0 * s).round() as i32;
        let bar_left = (cx - bar_w / 2.0).round() as i32;
        let bar_right = (cx + bar_w / 2.0).round() as i32;
        rect(rgba, bar_left, bar_top, bar_right, bar_bot, w, 255);
        // Dot.
        circle(rgba, cx, 12.5 * s, 1.0 * s, w, 255);
    }

    /// Lock body — used for permission missing.
    fn lock(rgba: &mut [u8], w: i32) {
        let s = w as f32 / 16.0;
        let cx = w as f32 / 2.0;
        let cy = w as f32 / 2.0 + 1.0 * s;
        // Lock body.
        let body_top = (cy - 2.5 * s).round() as i32;
        let body_bot = (cy + 3.5 * s).round() as i32;
        let body_left = (cx - 3.5 * s).round() as i32;
        let body_right = (cx + 3.5 * s).round() as i32;
        rect(rgba, body_left, body_top, body_right, body_bot, w, 255);

        // Shackle (the U-shape on top).
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
        // Overlay a corner badge: small bang.
        bang(&mut rgba, w);
        rgba
    }

    pub fn permission(w: i32) -> Vec<u8> {
        let mut rgba = vec![0u8; (w * w * 4) as usize];
        lock(&mut rgba, w);
        rgba
    }
}

fn write_png(path: &Path, rgba: &[u8], w: u32) {
    let file = fs::File::create(path).expect("create tray PNG");
    let buf = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(buf, w, w);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write png header");
    writer.write_image_data(rgba).expect("write png data");
}

fn emit_tray_icons() {
    // Always re-run if the build script changes.
    println!("cargo:rerun-if-changed=build.rs");

    let dir = Path::new("icons/tray");
    if !dir.exists() {
        fs::create_dir_all(dir).expect("create icons/tray");
    }

    type Draw = fn(i32) -> Vec<u8>;
    // (state, primary-frame, alt-frame-for-pulse)
    let states: &[(&str, Draw, Draw)] = &[
        ("idle", |w| tray_glyphs::idle(w), |w| tray_glyphs::idle(w)),
        // For "recording", the @1x and @2x are the static frame; the alt
        // (pulse-off) variant is the same mic without the dot. Both go to
        // disk so they're inspectable; runtime uses the in-memory generator.
        (
            "recording",
            |w| tray_glyphs::recording(w, true),
            |w| tray_glyphs::recording(w, false),
        ),
        (
            "processing",
            |w| tray_glyphs::processing(w),
            |w| tray_glyphs::processing(w),
        ),
        ("error", |w| tray_glyphs::error(w), |w| tray_glyphs::error(w)),
        (
            "permission",
            |w| tray_glyphs::permission(w),
            |w| tray_glyphs::permission(w),
        ),
    ];

    for (name, draw_a, _draw_b) in states {
        let p1 = dir.join(format!("{name}.png"));
        let p2 = dir.join(format!("{name}@2x.png"));
        write_png(&p1, &draw_a(16), 16);
        write_png(&p2, &draw_a(32), 32);
    }

    // Pulse-off variant for recording (mic without dot), so designers can
    // see what the alternate frame looks like. The runtime uses the same
    // procedural generator and doesn't read these files.
    write_png(
        &dir.join("recording-off.png"),
        &tray_glyphs::recording(16, false),
        16,
    );
    write_png(
        &dir.join("recording-off@2x.png"),
        &tray_glyphs::recording(32, false),
        32,
    );
}

/// Compile `swift/AppleFmBridge.swift` into a static library and emit the
/// link directives that pull it into the final binary.
///
/// Why a static lib (not a subprocess or a dylib): Foundation Models
/// brokers the heavy work to a system daemon (modelinferenced) so a
/// model crash can't crash the menu-bar app. That removes the
/// crash-isolation argument that drove `llama-server` out-of-process.
/// In-process gives us sub-millisecond bridge overhead instead of the
/// stdin/stdout JSON-LINES dance, and avoids shipping a second signed
/// binary in the .app bundle.
///
/// Deployment target is macOS 14.0 even though Foundation Models needs
/// macOS 26.0 — the framework gets weakly linked, so the binary still
/// loads on older Macs and the runtime `if #available(macOS 26, *)` in
/// `AppleFmBridge.swift` returns the right "OS too old" code.
#[cfg(target_os = "macos")]
fn build_apple_fm_bridge() -> Result<(), String> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").map_err(|e| format!("CARGO_MANIFEST_DIR: {e}"))?;
    let out_dir = env::var("OUT_DIR").map_err(|e| format!("OUT_DIR: {e}"))?;

    let swift_src = PathBuf::from(&manifest_dir).join("swift/AppleFmBridge.swift");
    if !swift_src.exists() {
        return Err(format!("missing swift source: {}", swift_src.display()));
    }
    println!("cargo:rerun-if-changed={}", swift_src.display());

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".into());
    let swift_target = match target_arch.as_str() {
        "aarch64" => "arm64-apple-macos14.0",
        "x86_64" => "x86_64-apple-macos14.0",
        other => return Err(format!("unsupported macOS arch: {other}")),
    };

    let lib_path = PathBuf::from(&out_dir).join("libapple_fm_bridge.a");
    let module_path = PathBuf::from(&out_dir).join("AppleFmBridge.swiftmodule");

    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let opt_flag = if profile == "release" { "-O" } else { "-Onone" };

    let mut cmd = Command::new("swiftc");
    cmd.args([
        "-emit-library",
        "-static",
        "-parse-as-library",
        opt_flag,
        "-target",
        swift_target,
        "-module-name",
        "AppleFmBridge",
        "-emit-module-path",
        &module_path.display().to_string(),
        "-o",
        &lib_path.display().to_string(),
        &swift_src.display().to_string(),
    ]);

    let output = cmd.output().map_err(|e| format!("invoke swiftc: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "swiftc failed (exit {}):\nstderr:\n{}\nstdout:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }

    println!("cargo:rustc-link-search=native={}", out_dir);
    println!("cargo:rustc-link-lib=static=apple_fm_bridge");

    // Foundation hard-linked. FoundationModels weak-linked because it
    // only exists on macOS 26 — must come through as adjacent linker
    // args, hence the split `-weak_framework` / framework-name pair.
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-arg=-weak_framework");
    println!("cargo:rustc-link-arg=FoundationModels");

    // Swift runtime ships with macOS at /usr/lib/swift since the 5.0
    // ABI-stability cutover, but the linker still needs the rpath so
    // dyld can resolve the embedded references at startup. Linking the
    // concurrency lib explicitly is required for `Task.detached` /
    // `await` plumbing in the bridge.
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

    Ok(())
}

fn main() {
    // On macOS, AXIsProcessTrusted lives in ApplicationServices.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
    }

    #[cfg(target_os = "macos")]
    {
        // Only build the Swift bridge when the apple-fm feature is on.
        // CI without Xcode SDK can opt out via
        // `--no-default-features --features local-engines`.
        if env::var("CARGO_FEATURE_APPLE_FM").is_ok() {
            if let Err(e) = build_apple_fm_bridge() {
                // Hard-fail rather than silently producing a binary that
                // links against a missing static lib. The error message
                // tells the user how to opt out.
                panic!(
                    "apple-fm bridge build failed:\n  {e}\n\nIf you don't have Xcode/Swift, build without it:\n  cargo build --no-default-features --features local-engines"
                );
            }
        }
    }

    emit_tray_icons();
    tauri_build::build()
}
